/**
 * PixiJS 3D 后端适配器（@pixi/3d open beta 0.1.0）。
 *
 * 与 three 端 [`Stage`](./stage.ts) 对外暴露同一套方法面，供 App.vue 在
 * three / pixi 两种引擎之间切换。数据路径复用 SDK 解析出的 `.l2d3` 模型：
 * 每网格一份静态参考帧几何 + FlatMaterial，逐帧把烘焙的列主序 4×4 世界矩阵
 * 分解成平移 / 四元数 / 缩放写回 `Mesh3D` 的分量（Container3D 每帧会从
 * 分量重建 localTransform，不能直写矩阵）。
 *
 * **当前退化项**（open beta 阶段先打通可切换骨架，与 three/WebGPU 后端一致的
 * 分阶段策略）：
 * - 逐帧顶点（framePositions / frameOffsets / frameBuckets 多桶切换）不重建几何，
 *   仅取参考帧静态顶点；
 * - 渐变 LUT、柔光高斯模糊、选择框 / 悬停高亮、射线拾取、节点重排、层深 / 厚度
 *   滑块在本后端暂为空操作；
 * - 正交取景沿用 three 端的半高 2.62、相机位于 +Z 7.2。
 */
import { Application } from 'pixi.js';
import {
  Camera3D,
  FlatMaterial,
  Geometry3D,
  Matrix4,
  Mesh3D,
  Quaternion,
  Vector3,
  View3D,
} from '@pixi/3d';
import { OrbitCamera } from '@pixi/3d/extras';
import type { RendererBackend, StageCallbacks, ViewState } from './stage';
import type { Model } from '../../sdk/index.mjs';

/** 与 three 端一致的正交取景半高（世界单位）。 */
const ORTHO_HALF_HEIGHT = 2.62;
/** 默认相机位置（正对 +Z）。 */
const HOME_Z = 7.2;

/** 单个网格在 Pixi 后端的运行时记录。 */
interface PixiEntry {
  /** 3D 网格；参考帧几何为空时为 null（占位以保持 id 对齐）。 */
  mesh: Mesh3D | null;
  /** 材质（与 mesh 同生命周期）。 */
  material: FlatMaterial | null;
  /** 逐帧世界矩阵轨道（frameCount × 16，列主序）；常量轨道为 null。 */
  track: Float32Array | null;
  /** 逐帧 RGBA 颜色轨道；无逐帧颜色为 null。 */
  colors: Float32Array | null;
  /** 参考帧基色（sRGB 0..1，RGB 三项）。 */
  baseColor: [number, number, number];
  /** 参考帧不透明度。 */
  baseAlpha: number;
  /** 节点被 UI 隐藏。 */
  hidden: boolean;
}

/** sRGB 0..1 三项转 0xRRGGBB 整数。 */
function rgbToHex(r: number, g: number, b: number): number {
  const clamp = (v: number): number => Math.max(0, Math.min(1, v));
  return (
    (Math.round(clamp(r) * 255) << 16) |
    (Math.round(clamp(g) * 255) << 8) |
    Math.round(clamp(b) * 255)
  );
}

export class PixiStage {
  private readonly host: HTMLElement;
  private readonly callbacks: StageCallbacks;
  private app: Application | null = null;
  private view: View3D | null = null;
  private orbit: OrbitCamera | null = null;
  private model: Model | null = null;
  private entries: PixiEntry[] = [];
  private frameIndex = 0;
  private autoRotateWanted = false;
  private rotationLocked = false;
  private disposed = false;

  // 复用的矩阵 / 分量临时对象，避免逐帧分配。
  private readonly scratchMatrix = new Matrix4();
  private readonly scratchPos = new Vector3();
  private readonly scratchQuat = new Quaternion();
  private readonly scratchScale = new Vector3();

  private constructor(host: HTMLElement, callbacks: StageCallbacks = {}) {
    this.host = host;
    this.callbacks = callbacks;
  }

  /** 异步装配：Application 初始化必须 await，再挂 View3D 与环绕相机。 */
  static async create(host: HTMLElement, callbacks: StageCallbacks = {}): Promise<PixiStage> {
    const stage = new PixiStage(host, callbacks);
    await stage.init();
    return stage;
  }

  private async init(): Promise<void> {
    const app = new Application();
    await app.init({
      resizeTo: this.host,
      antialias: true,
      backgroundAlpha: 0,
      autoDensity: true,
      preference: 'webgl',
    });
    this.app = app;
    this.host.appendChild(app.canvas);

    const view = new View3D({
      autoResize: true,
      cull: 'off',
      background: null,
      toneMapping: 'none',
    });
    this.view = view;
    app.stage.addChild(view);

    const camera: Camera3D = view.camera;
    // Pixi 的 orthographicSize 是「半宽」，高度随宽高比反推；three 端给定的是半高
    // 2.62，故半宽取 2.62 × 宽高比，反推后半高仍为 2.62，两侧取景一致。
    const hostW = Math.max(1, this.host.clientWidth);
    const hostH = Math.max(1, this.host.clientHeight);
    const aspect = hostW / hostH;
    camera.useOrthographic(ORTHO_HALF_HEIGHT * aspect, -20, 40);
    camera.position.set(0, 0, HOME_Z);

    this.orbit = new OrbitCamera({
      view,
      target: { x: 0, y: 0, z: 0 },
      enableDamping: true,
      minDistance: 2.4,
      maxDistance: 18,
      autoRotate: false,
      autoRotateSpeed: 0.65,
    });
  }

  /** 建立网格：参考帧静态几何 + 不受光照的 FlatMaterial。 */
  setModel(model: Model): void {
    this.clearEntries();
    this.model = model;
    const root = this.view?.root;
    if (root == null) {
      return;
    }

    for (const data of model.meshes) {
      const track = model.transforms[data.transformIndex] ?? null;
      const colors = data.colors;
      const baseColor: [number, number, number] = [data.color[0], data.color[1], data.color[2]];
      const baseAlpha = data.color[3];

      let mesh: Mesh3D | null = null;
      let material: FlatMaterial | null = null;

      const positions = data.positions;
      const bucket = data.buckets && data.buckets.length > 0 ? data.buckets[0] : null;
      const indices = bucket ? bucket.indices : data.indices;
      if (positions != null && positions.length >= 3 && indices != null && indices.length > 0) {
        const geometry = new Geometry3D({ positions, indices });
        material = new FlatMaterial({
          baseColor: rgbToHex(baseColor[0], baseColor[1], baseColor[2]),
          alpha: baseAlpha,
          alphaMode: baseAlpha < 0.999 ? 'blend' : 'opaque',
          doubleSided: true,
        });
        mesh = new Mesh3D({ geometry, material });
        root.addChild(mesh);
      }

      this.entries.push({
        mesh,
        material,
        track,
        colors,
        baseColor,
        baseAlpha,
        hidden: false,
      });
    }

    this.applyFrame(Math.min(this.frameIndex, Math.max(0, model.frameCount - 1)));
  }

  /** 切帧：写世界矩阵分量与逐帧颜色；逐帧顶点桶切换在本后端退化。 */
  setFrame(frame: number): void {
    if (this.model == null) {
      return;
    }
    const clamped = Math.max(0, Math.min(this.model.frameCount - 1, Math.round(frame)));
    if (clamped === this.frameIndex) {
      return;
    }
    this.applyFrame(clamped);
  }

  private applyFrame(frame: number): void {
    if (this.model == null) {
      return;
    }
    this.frameIndex = frame;

    for (const entry of this.entries) {
      const { mesh, material, track, colors } = entry;
      if (mesh == null || material == null) {
        continue;
      }

      if (entry.hidden) {
        mesh.visible = false;
        continue;
      }

      // 世界矩阵：从轨道读列主序 16 项，分解成平移 / 四元数 / 缩放写回分量。
      if (track != null) {
        this.scratchMatrix.fromArray(track, frame * 16);
        this.scratchMatrix.decompose(this.scratchPos, this.scratchQuat, this.scratchScale);
        mesh.position.copyFrom(this.scratchPos);
        mesh.quaternion.copyFrom(this.scratchQuat);
        // 退化 / 镜面矩阵可能给出 0 缩放，兜底避免被压扁到不可见。
        const sx = Math.abs(this.scratchScale.x) < 1e-6 ? 1 : this.scratchScale.x;
        const sy = Math.abs(this.scratchScale.y) < 1e-6 ? 1 : this.scratchScale.y;
        const sz = Math.abs(this.scratchScale.z) < 1e-6 ? 1 : this.scratchScale.z;
        mesh.scale.set(sx, sy, sz);
      } else {
        mesh.position.set(0, 0, 0);
        mesh.quaternion.set(0, 0, 0, 1);
        mesh.scale.set(1, 1, 1);
      }

      // 逐帧颜色：alpha 过低整帧不可见。
      if (colors != null) {
        const colorBase = frame * 4;
        const alpha = colors[colorBase + 3];
        if (alpha < 0.004) {
          mesh.visible = false;
          continue;
        }
        material.alpha = alpha;
        material.baseColor = rgbToHex(
          colors[colorBase],
          colors[colorBase + 1],
          colors[colorBase + 2],
        );
        material.alphaMode = 'blend';
        mesh.visible = true;
      } else {
        mesh.visible = entry.baseAlpha >= 0.004;
      }
    }
  }

  // ---- 与 three Stage 对齐的对外方法面；不支持的能力为空操作 ----

  /** 层深倍率：Pixi 后端暂不重放矩阵 Z 增量（退化）。 */
  setDepthScale(_value: number): void {}

  /** 厚度倍率：Pixi 后端暂不改写 Z 基向量（退化）。 */
  setThicknessScale(_value: number): void {}

  setAutoRotate(enabled: boolean): void {
    this.autoRotateWanted = enabled && !this.rotationLocked;
    if (this.orbit != null) {
      this.orbit.autoRotate = this.autoRotateWanted;
    }
  }

  setRotationLock(locked: boolean): void {
    this.rotationLocked = locked;
    if (this.orbit != null) {
      this.orbit.autoRotate = locked ? false : this.autoRotateWanted;
    }
  }

  /** 柔光总开关：Pixi 后端暂无后处理柔光（退化）。 */
  setSoftGlow(_enabled: boolean): void {}

  setNodeGlow(_id: number, _enabled: boolean): void {}

  setAllNodeGlow(_enabled: boolean): void {}

  setGlowIntensity(_value: number): void {}

  setGlowRadius(_value: number): void {}

  /** 选择高亮：Pixi 后端暂无选择框（退化）。 */
  setSelection(_ids: readonly number[]): void {}

  setHoverNode(_id: number | null): void {}

  setNodeVisible(id: number, visible: boolean): void {
    const entry = this.entries[id];
    if (entry == null) {
      return;
    }
    entry.hidden = !visible;
    if (entry.mesh != null) {
      entry.mesh.visible = visible && entry.baseAlpha >= 0.004;
    }
  }

  /** 节点重排：Pixi 后端按烘焙渲染顺序绘制（退化）。 */
  setNodeOrder(_ids: readonly number[]): void {}

  /** 射线拾取：open beta 阶段暂不接入，返回空（退化）。 */
  pickNodeAt(_clientX: number, _clientY: number): number | null {
    return null;
  }

  resetView(): void {
    this.orbit?.reset();
  }

  /** 快照当前相机位置、观察目标与缩放，供重建舞台后还原。 */
  getViewState(): ViewState {
    const cam = this.orbit?.camera;
    const target = this.orbit?.target;
    return {
      position: cam ? [cam.position.x, cam.position.y, cam.position.z] : [0, 0, HOME_Z],
      target: target ? [target.x, target.y, target.z] : [0, 0, 0],
      zoom: cam?.zoom ?? 1,
    };
  }

  /** 还原相机位置、观察目标与缩放。 */
  applyViewState(state: ViewState): void {
    if (this.orbit == null) {
      return;
    }
    this.orbit.target.set(state.target[0], state.target[1], state.target[2]);
    this.orbit.camera.position.set(state.position[0], state.position[1], state.position[2]);
    this.orbit.camera.zoom = state.zoom;
    this.orbit.update();
  }

  /** 分辨率自适应：Pixi 后端由 Application 自身 DPR 管理（退化）。 */
  adaptQuality(_fps: number): void {}

  /** Pixi 由内置 ticker 驱动渲染，无需手动 render。 */
  render(): void {}

  getBackend(): RendererBackend {
    return 'webgl';
  }

  private clearEntries(): void {
    for (const entry of this.entries) {
      entry.mesh?.destroy({ geometry: true, material: true });
    }
    this.entries = [];
  }

  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    this.clearEntries();
    this.orbit?.destroy();
    this.orbit = null;
    this.view?.destroy({ children: true });
    this.view = null;
    this.app?.destroy(true, { children: true });
    this.app = null;
    this.model = null;
    // 回调在销毁后不再触发。
    void this.callbacks;
  }
}
