import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import type { Model, MeshModel } from '../../sdk/index.mjs';

/** Six Blender-style view directions: +X, -X, +Y, -Y, +Z, -Z. */
export type ViewAxis = 'x' | '-x' | 'y' | '-y' | 'z' | '-z';

export interface StageCallbacks {
  onAxisPick?: (axis: ViewAxis) => void;
}

/** One selectable mesh as exposed to the UI / external API. */
export interface StageNode {
  id: number;
  name: string;
  kind: number;
  animated: boolean;
  vertexCount: number;
  indexCount: number;
}

export const AXIS_LABELS: Record<ViewAxis, string> = {
  x: '+X',
  '-x': '-X',
  y: '+Y',
  '-y': '-Y',
  z: '+Z',
  '-z': '-Z',
};

const AXIS_ORDER: ViewAxis[] = ['x', '-x', 'y', '-y', 'z', '-z'];

const AXIS_VECTORS: Record<ViewAxis, THREE.Vector3> = {
  x: new THREE.Vector3(1, 0, 0),
  '-x': new THREE.Vector3(-1, 0, 0),
  y: new THREE.Vector3(0, 1, 0),
  '-y': new THREE.Vector3(0, -1, 0),
  z: new THREE.Vector3(0, 0, 1),
  '-z': new THREE.Vector3(0, 0, -1),
};

const AXIS_COLORS: Record<ViewAxis, number> = {
  x: 0xff6b6b,
  '-x': 0x9c4a52,
  y: 0x7dff9e,
  '-y': 0x45875a,
  z: 0x6ea8ff,
  '-z': 0x466a9f,
};

/** Default view: straight on +Z (Blender front view). */
const AXIS_HOME = new THREE.Vector3(0, 0, 7.2);
const VIEW_TWEEN_MS = 320;

const SELECTION_COLOR = 0x3d8bff;
const GLOW_SCALE = 0.25;
const GLOW_OPACITY = 0.8;

const FULLSCREEN_VERTEX = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

const GAUSSIAN_BLUR = `
uniform sampler2D source;
uniform vec2 direction;
varying vec2 vUv;
void main() {
  vec4 sum = texture2D(source, vUv) * 0.2270270270;
  sum += texture2D(source, vUv + direction * 1.3846153846) * 0.3162162162;
  sum += texture2D(source, vUv - direction * 1.3846153846) * 0.3162162162;
  sum += texture2D(source, vUv + direction * 3.2307692308) * 0.0702702703;
  sum += texture2D(source, vUv - direction * 3.2307692308) * 0.0702702703;
  gl_FragColor = sum;
}
`;

interface Entry {
  data: MeshModel;
  mesh: THREE.Mesh;
  geometry: THREE.BufferGeometry;
  material: THREE.MeshLambertMaterial;
  /** Constant base matrix for statically transformed layers. */
  base: THREE.Matrix4;
  track: Float32Array | null;
  transformStatic: boolean;
  colors: Float32Array | null;
  position: THREE.BufferAttribute;
  positionsAnimated: boolean;
  /** Visibility derived from the base alpha, before manual hiding. */
  staticVisible: boolean;
}

/** Tracks that never change only need one matrix upload, not one per frame. */
function isConstantTrack(track: Float32Array, frameCount: number): boolean {
  if (frameCount <= 1) {
    return true;
  }
  for (let frame = 1; frame < frameCount; frame += 1) {
    const offset = frame * 16;
    for (let i = 0; i < 16; i += 1) {
      if (track[offset + i] !== track[i]) {
        return false;
      }
    }
  }
  return true;
}

/**
 * three.js stage for the Rust-compiled model.
 *
 * Every mesh keeps a fixed BufferGeometry; per frame we only copy baked vertex
 * positions (animated shapes) and write the world matrix directly, skipping
 * any parent multiplication or work on layers that never move.
 */
export class Stage {
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera: THREE.PerspectiveCamera;
  private readonly controls: OrbitControls;
  private readonly host: HTMLElement;
  private readonly entries: Entry[] = [];
  private model: Model | null = null;
  private frameIndex = -1;
  private dirty = true;
  private depthAmount = 1;
  private thicknessAmount = 1;
  private pixelRatio = 1.5;
  private lowFpsWindows = 0;
  private readonly observer: ResizeObserver;
  private readonly callbacks: StageCallbacks;
  private readonly nodes: StageNode[] = [];

  /** Per-node selection wires (blue 1px cuboids). */
  private readonly selectionBoxes = new Map<number, THREE.Box3Helper>();
  private readonly boxScratch = new THREE.Box3();

  /** Node ids hidden from the scene through the node list. */
  private readonly hiddenNodes = new Set<number>();

  /** Soft glow: per-node silhouettes blurred and added over the main image. */
  private softGlow = false;
  private readonly glowFlags = new Map<number, boolean>();
  private readonly glowMeshes = new Map<number, THREE.Mesh>();
  private readonly glowScene = new THREE.Scene();
  private glowRead: THREE.WebGLRenderTarget | null = null;
  private glowWrite: THREE.WebGLRenderTarget | null = null;
  private glowWidth = 0;
  private glowHeight = 0;
  private glowCompositeMaterial: THREE.MeshBasicMaterial | null = null;
  private readonly glowQuadScene = new THREE.Scene();
  private readonly glowQuadCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  private readonly glowQuadGeometry = new THREE.PlaneGeometry(2, 2);
  private readonly glowQuad = new THREE.Mesh(this.glowQuadGeometry);
  private readonly glowBlurMaterial = new THREE.ShaderMaterial({
    vertexShader: FULLSCREEN_VERTEX,
    fragmentShader: GAUSSIAN_BLUR,
    uniforms: {
      source: { value: null },
      direction: { value: new THREE.Vector2(1, 0) },
    },
    depthTest: false,
    depthWrite: false,
  });

  /** Blender-style orientation gizmo rendered in the top-left corner. */
  private readonly gizmoScene = new THREE.Scene();
  private readonly gizmoCamera = new THREE.OrthographicCamera(-1.6, 1.6, 1.6, -1.6, 0.1, 20);
  private readonly gizmoBalls = new Map<ViewAxis, THREE.Mesh>();
  private readonly gizmoLabels = new Map<ViewAxis, THREE.Sprite>();
  private readonly gizmoTextures: THREE.Texture[] = [];
  private readonly gizmoResources: Array<{ dispose(): void }> = [];
  private readonly gizmoProjection = new THREE.Vector3();
  private gizmoSelection: THREE.Sprite | null = null;
  private selectedAxis: ViewAxis | null = null;
  private rotationLocked = false;
  private viewTween: { from: THREE.Vector3; to: THREE.Vector3; started: number } | null = null;
  private gizmoPointerDown: { x: number; y: number } | null = null;
  private readonly gizmoMargin = 12;
  private readonly gizmoDirection = new THREE.Vector3();

  private readonly onGizmoPointerDown = (event: PointerEvent): void => {
    if (event.button !== 0) {
      return;
    }
    const point = this.localPoint(event);
    if (!this.inGizmo(point)) {
      return;
    }
    event.stopPropagation();
    event.preventDefault();
    this.gizmoPointerDown = point;
  };

  private readonly onGizmoPointerUp = (event: PointerEvent): void => {
    const start = this.gizmoPointerDown;
    if (start == null) {
      return;
    }
    this.gizmoPointerDown = null;
    const point = this.localPoint(event);
    event.stopPropagation();
    event.preventDefault();
    if (Math.hypot(point.x - start.x, point.y - start.y) > 6) {
      return;
    }
    const axis = this.pickGizmoAxis(start);
    if (axis == null) {
      return;
    }
    this.viewAxis(axis);
    this.callbacks.onAxisPick?.(axis);
  };

  constructor(host: HTMLElement, callbacks: StageCallbacks = {}) {
    this.host = host;
    this.callbacks = callbacks;
    this.renderer = new THREE.WebGLRenderer({
      antialias: true,
      alpha: true,
      powerPreference: 'high-performance',
    });
    this.pixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
    this.renderer.setPixelRatio(this.pixelRatio);
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.autoClear = false;
    this.renderer.domElement.className = 'three-canvas';
    host.appendChild(this.renderer.domElement);

    this.camera = new THREE.PerspectiveCamera(40, 1, 0.1, 60);
    this.camera.position.copy(AXIS_HOME);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.target.set(0, 0, 0);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.09;
    this.controls.rotateSpeed = 0.85;
    this.controls.minDistance = 2.4;
    this.controls.maxDistance = 18;
    this.controls.autoRotate = false;
    this.controls.autoRotateSpeed = 0.65;
    this.controls.addEventListener('change', () => {
      this.dirty = true;
    });

    const ambient = new THREE.AmbientLight(0xffffff, 1.25);
    const key = new THREE.DirectionalLight(0xdfe9ff, 2.4);
    key.position.set(2.4, 3.2, 4.2);
    const rim = new THREE.DirectionalLight(0xffb98a, 1.6);
    rim.position.set(-3.4, -1.6, -2.2);
    const fill = new THREE.DirectionalLight(0x86a8ff, 0.9);
    fill.position.set(-1.5, -2.8, 3.4);
    this.scene.add(ambient, key, rim, fill);

    this.buildGizmo();
    this.glowQuad.frustumCulled = false;
    this.glowQuadScene.add(this.glowQuad);
    // Capture on the container so the gizmo click is handled before
    // OrbitControls' canvas listeners start a drag.
    host.addEventListener('pointerdown', this.onGizmoPointerDown, true);
    host.addEventListener('pointerup', this.onGizmoPointerUp, true);

    this.observer = new ResizeObserver(() => this.resize());
    this.observer.observe(host);
    this.resize();
  }

  get hostElement(): HTMLElement {
    return this.host;
  }

  setModel(model: Model): void {
    this.clear();
    this.model = model;
    this.frameIndex = -1;
    this.nodes.length = 0;
    this.glowFlags.clear();

    // Meshes of the same layer share a transform track; detect statically
    // transformed layers only once per track.
    const staticTracks = new Map<number, boolean>();

    for (const [id, data] of model.meshes.entries()) {
      const positionsAnimated = data.animated && data.framePositions != null;
      const positions = positionsAnimated
        ? new Float32Array(data.positions)
        : data.positions;

      const geometry = new THREE.BufferGeometry();
      const position = new THREE.BufferAttribute(positions, 3);
      position.setUsage(positionsAnimated ? THREE.DynamicDrawUsage : THREE.StaticDrawUsage);
      geometry.setAttribute('position', position);
      geometry.setIndex(new THREE.BufferAttribute(data.indices, 1));

      const track = model.transforms[data.transformIndex] ?? null;
      let transformStatic = staticTracks.get(data.transformIndex);
      if (transformStatic === undefined) {
        transformStatic = track == null || isConstantTrack(track, model.frameCount);
        staticTracks.set(data.transformIndex, transformStatic);
      }

      const colors = data.colors;
      const alpha = colors ? colors[3] : data.color[3];
      // Lottie colours are sRGB; convert once here so static and animated
      // colours share the same linear-space representation as three.js.
      const baseColor = new THREE.Color().setRGB(
        data.color[0],
        data.color[1],
        data.color[2],
        THREE.SRGBColorSpace,
      );
      const material = new THREE.MeshLambertMaterial({
        color: baseColor,
        flatShading: true,
        side: THREE.DoubleSide,
        // Opaque constant colours skip blending and per-frame sorting entirely.
        transparent: colors != null || alpha < 0.999,
        opacity: Math.max(0, Math.min(1, alpha)),
      });

      const mesh = new THREE.Mesh(geometry, material);
      mesh.matrixAutoUpdate = false;
      mesh.matrixWorldAutoUpdate = false;
      mesh.matrixWorldNeedsUpdate = false;
      mesh.frustumCulled = false;
      mesh.renderOrder = 0;
      const staticVisible = alpha >= 0.004;
      mesh.visible = staticVisible;
      this.scene.add(mesh);

      const base = new THREE.Matrix4();
      if (track) {
        base.fromArray(track, 0);
      }

      this.entries.push({
        data,
        mesh,
        geometry,
        material,
        base,
        track,
        transformStatic,
        colors,
        position,
        positionsAnimated,
        staticVisible,
      });
      this.nodes.push({
        id,
        name: data.name || `节点 ${id + 1}`,
        kind: data.kind,
        animated: data.animated,
        vertexCount: data.vertexCount,
        indexCount: data.indexCount,
      });
    }

    this.applyFrame(0);
    this.syncGlowMeshes();
    this.dirty = true;
  }

  /** Node list in render order; ids match `setSelection` ids. */
  getNodes(): readonly StageNode[] {
    return this.nodes;
  }

  /** Selected node ids; also drives the blue wire cuboids. */
  setSelection(ids: readonly number[]): void {
    const wanted = new Set<number>();
    for (const id of ids) {
      if (id >= 0 && id < this.entries.length) {
        wanted.add(id);
      }
    }

    for (const [id, helper] of this.selectionBoxes) {
      if (!wanted.has(id)) {
        this.scene.remove(helper);
        helper.geometry.dispose();
        (helper.material as THREE.Material).dispose();
        this.selectionBoxes.delete(id);
      }
    }

    for (const id of wanted) {
      if (this.selectionBoxes.has(id)) {
        continue;
      }
      const helper = new THREE.Box3Helper(new THREE.Box3(), SELECTION_COLOR);
      (helper.material as THREE.LineBasicMaterial).depthTest = false;
      helper.renderOrder = 12;
      helper.visible = false;
      this.scene.add(helper);
      this.selectionBoxes.set(id, helper);
    }

    this.updateSelectionBoxes();
    this.dirty = true;
  }

  /** Toggle the global soft-glow pass. */
  setSoftGlow(enabled: boolean): void {
    if (this.softGlow === enabled) {
      return;
    }
    this.softGlow = enabled;
    this.syncGlowMeshes();
    this.dirty = true;
  }

  /** Enable / disable glow for a single node. */
  setNodeGlow(id: number, enabled: boolean): void {
    if (this.glowFlags.get(id) === enabled) {
      return;
    }
    this.glowFlags.set(id, enabled);
    this.syncGlowMeshes();
    this.dirty = true;
  }

  /** Apply the master switch to every node at once. */
  setAllNodeGlow(enabled: boolean): void {
    let changed = false;
    for (let id = 0; id < this.entries.length; id += 1) {
      if (this.glowFlags.get(id) !== enabled) {
        this.glowFlags.set(id, enabled);
        changed = true;
      }
    }
    if (!changed) {
      return;
    }
    this.syncGlowMeshes();
    this.dirty = true;
  }

  /** Show or hide one node; hidden nodes also drop their selection wire. */
  setNodeVisible(id: number, visible: boolean): void {
    const hidden = !visible;
    if (id < 0 || id >= this.entries.length || this.hiddenNodes.has(id) === hidden) {
      return;
    }
    if (hidden) {
      this.hiddenNodes.add(id);
    } else {
      this.hiddenNodes.delete(id);
    }
    this.applyFrame(Math.max(0, this.frameIndex));
    this.updateSelectionBoxes();
    this.dirty = true;
  }

  setFrame(frame: number): void {
    if (this.model == null) {
      return;
    }
    const clamped = Math.max(0, Math.min(this.model.frameCount - 1, Math.round(frame)));
    if (clamped === this.frameIndex) {
      return;
    }
    this.applyFrame(clamped);
    this.dirty = true;
  }

  setDepthScale(value: number): void {
    this.depthAmount = value;
    if (this.model != null) {
      this.applyFrame(Math.max(0, this.frameIndex));
      this.dirty = true;
    }
  }

  setThicknessScale(value: number): void {
    this.thicknessAmount = value;
    if (this.model != null) {
      this.applyFrame(Math.max(0, this.frameIndex));
      this.dirty = true;
    }
  }

  setAutoRotate(enabled: boolean): void {
    this.controls.autoRotate = enabled && !this.rotationLocked;
    this.dirty = true;
  }

  /** Lock orbiting while keeping pan / zoom and axis switching available. */
  setRotationLock(locked: boolean): void {
    this.rotationLocked = locked;
    this.controls.enableRotate = !locked;
    if (locked) {
      this.controls.autoRotate = false;
    }
    this.updateGizmoSelection();
    this.dirty = true;
  }

  /** Snap the camera to one of the six Blender-style view directions. */
  viewAxis(axis: ViewAxis): void {
    const distance = Math.max(2.4, this.camera.position.distanceTo(this.controls.target));
    const to = this.controls.target.clone().add(AXIS_VECTORS[axis].clone().multiplyScalar(distance));
    // OrbitControls' spherical conversion degenerates exactly on the Y pole,
    // so top / bottom views get a fraction of a degree of tilt.
    if (Math.abs(AXIS_VECTORS[axis].y) > 0.5) {
      to.z += distance * 0.004;
    }
    this.selectedAxis = axis;
    this.viewTween = {
      from: this.camera.position.clone(),
      to,
      started: performance.now(),
    };
    this.updateGizmoSelection();
    this.dirty = true;
  }

  /**
   * Lower the render resolution when playback keeps missing the frame budget.
   * Called from the FPS readout, so it reacts within a few seconds and never
   * costs anything while the scene is smooth.
   */
  adaptQuality(fps: number): void {
    if (fps >= 45 || this.pixelRatio <= 0.8) {
      this.lowFpsWindows = 0;
      return;
    }
    this.lowFpsWindows += 1;
    if (this.lowFpsWindows < 6) {
      return;
    }
    this.lowFpsWindows = 0;
    this.pixelRatio = Math.max(0.75, this.pixelRatio * 0.8);
    this.renderer.setPixelRatio(this.pixelRatio);
    this.resize();
  }

  resetView(): void {
    this.selectedAxis = null;
    this.viewTween = null;
    this.camera.position.copy(AXIS_HOME);
    this.controls.target.set(0, 0, 0);
    this.controls.update();
    this.updateGizmoSelection();
    this.dirty = true;
  }

  render(): void {
    const now = performance.now();
    if (this.viewTween) {
      const t = Math.min(1, (now - this.viewTween.started) / VIEW_TWEEN_MS);
      const eased = 1 - (1 - t) ** 3;
      this.camera.position.lerpVectors(this.viewTween.from, this.viewTween.to, eased);
      if (t >= 1) {
        this.viewTween = null;
      }
      this.dirty = true;
    }

    const moved = this.controls.update();
    if (!moved && !this.dirty) {
      return;
    }

    const width = Math.max(1, this.host.clientWidth);
    const height = Math.max(1, this.host.clientHeight);
    this.updateSelectionBoxes();
    this.renderer.setViewport(0, 0, width, height);
    this.renderer.setScissorTest(false);
    this.renderer.clear();
    this.renderer.render(this.scene, this.camera);
    this.renderGlow(width, height);
    this.renderGizmo(width, height);
    this.dirty = false;
  }

  dispose(): void {
    this.observer.disconnect();
    this.host.removeEventListener('pointerdown', this.onGizmoPointerDown, true);
    this.host.removeEventListener('pointerup', this.onGizmoPointerUp, true);
    this.clear();
    this.disposeGlow();
    this.disposeGizmo();
    this.controls.dispose();
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }

  private applyFrame(frame: number): void {
    if (this.model == null) {
      return;
    }
    this.frameIndex = frame;
    const frameCount = this.model.frameCount;
    const trackOffset = Math.min(frame, frameCount - 1) * 16;
    const depthDelta = this.depthAmount - 1;
    const thickness = this.thicknessAmount;

    for (let id = 0; id < this.entries.length; id += 1) {
      const entry = this.entries[id];
      const { data, mesh, material, position, colors, track } = entry;

      // Hidden nodes skip all per-frame work until the eye is toggled back on.
      if (this.hiddenNodes.has(id)) {
        mesh.visible = false;
        continue;
      }

      // Write the world matrix directly: layers sit under the scene root, so
      // there is no parent multiply to do and nothing to invalidate.
      if (entry.transformStatic) {
        mesh.matrixWorld.copy(entry.base);
      } else if (track != null) {
        mesh.matrixWorld.fromArray(track, trackOffset);
      }
      const elements = mesh.matrixWorld.elements;
      if (depthDelta !== 0) {
        // Depth is baked into the track, so the slider only adds a delta.
        elements[14] += data.depth * depthDelta;
      }
      if (thickness !== 1) {
        // Equivalent to post-multiplying by scale(1, 1, thickness).
        elements[2] *= thickness;
        elements[6] *= thickness;
        elements[10] *= thickness;
      }

      if (colors != null) {
        const colorBase = frame * 4;
        const alpha = colors[colorBase + 3];
        if (alpha < 0.004) {
          mesh.visible = false;
          continue;
        }
        mesh.visible = true;
        material.opacity = alpha;
        material.color.setRGB(
          colors[colorBase],
          colors[colorBase + 1],
          colors[colorBase + 2],
          THREE.SRGBColorSpace,
        );
      } else {
        mesh.visible = entry.staticVisible;
      }

      if (entry.positionsAnimated && data.framePositions) {
        const count = data.vertexCount * 3;
        const offset = frame * count;
        if (offset + count <= data.framePositions.length) {
          position.array.set(data.framePositions.subarray(offset, offset + count));
          position.needsUpdate = true;
        }
      }
    }
  }

  private clear(): void {
    for (const helper of this.selectionBoxes.values()) {
      this.scene.remove(helper);
      helper.geometry.dispose();
      (helper.material as THREE.Material).dispose();
    }
    this.selectionBoxes.clear();
    this.clearGlowMeshes();

    for (const entry of this.entries) {
      this.scene.remove(entry.mesh);
      entry.geometry.dispose();
      entry.material.dispose();
    }
    this.entries.length = 0;
    this.nodes.length = 0;
    this.hiddenNodes.clear();
    this.model = null;
  }

  private resize(): void {
    const width = Math.max(1, this.host.clientWidth);
    const height = Math.max(1, this.host.clientHeight);
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.dirty = true;
  }

  // ------------------------------------------------------- selection & glow

  private updateSelectionBoxes(): void {
    for (const [id, helper] of this.selectionBoxes) {
      const entry = this.entries[id];
      if (entry == null || !entry.mesh.visible) {
        helper.visible = false;
        continue;
      }
      this.boxScratch.setFromBufferAttribute(entry.position);
      this.boxScratch.applyMatrix4(entry.mesh.matrixWorld);
      if (this.boxScratch.isEmpty()) {
        helper.visible = false;
        continue;
      }
      helper.box.copy(this.boxScratch);
      helper.visible = true;
    }
  }

  private isNodeGlowEnabled(id: number): boolean {
    // With the master pass on every node glows unless it was explicitly
    // excluded; with it off only nodes explicitly enabled still glow, so a
    // single node can light up on its own.
    return this.softGlow
      ? this.glowFlags.get(id) !== false
      : this.glowFlags.get(id) === true;
  }

  /** Create / drop glow shells so only enabled nodes take part in the pass. */
  private syncGlowMeshes(): void {
    const wanted = new Set<number>();
    for (let id = 0; id < this.entries.length; id += 1) {
      if (this.isNodeGlowEnabled(id)) {
        wanted.add(id);
      }
    }

    for (const [id, mesh] of this.glowMeshes) {
      if (wanted.has(id)) {
        continue;
      }
      this.glowScene.remove(mesh);
      (mesh.material as THREE.Material).dispose();
      this.glowMeshes.delete(id);
    }

    for (const id of wanted) {
      if (this.glowMeshes.has(id)) {
        continue;
      }
      const entry = this.entries[id];
      if (entry == null) {
        continue;
      }
      const material = new THREE.MeshBasicMaterial({
        transparent: true,
        blending: THREE.AdditiveBlending,
        depthTest: false,
        depthWrite: false,
        toneMapped: false,
      });
      const mesh = new THREE.Mesh(entry.geometry, material);
      mesh.matrixAutoUpdate = false;
      mesh.matrixWorldAutoUpdate = false;
      mesh.frustumCulled = false;
      this.glowScene.add(mesh);
      this.glowMeshes.set(id, mesh);
    }

    this.dirty = true;
  }

  private clearGlowMeshes(): void {
    for (const mesh of this.glowMeshes.values()) {
      this.glowScene.remove(mesh);
      (mesh.material as THREE.Material).dispose();
    }
    this.glowMeshes.clear();
  }

  private ensureGlowTargets(width: number, height: number): boolean {
    const targetWidth = Math.max(64, Math.floor(width * GLOW_SCALE));
    const targetHeight = Math.max(64, Math.floor(height * GLOW_SCALE));
    if (
      this.glowRead != null &&
      this.glowWrite != null &&
      this.glowWidth === targetWidth &&
      this.glowHeight === targetHeight
    ) {
      return true;
    }

    this.glowRead?.dispose();
    this.glowWrite?.dispose();
    const options = {
      minFilter: THREE.LinearFilter,
      magFilter: THREE.LinearFilter,
      depthBuffer: false,
      stencilBuffer: false,
    };
    this.glowRead = new THREE.WebGLRenderTarget(targetWidth, targetHeight, options);
    this.glowWrite = new THREE.WebGLRenderTarget(targetWidth, targetHeight, options);
    this.glowWidth = targetWidth;
    this.glowHeight = targetHeight;
    return true;
  }

  /**
   * Additive soft glow: render only the enabled meshes, blur the result at a
   * quarter resolution and composite it over the already-rendered scene.
   */
  private renderGlow(width: number, height: number): void {
    // Runs whenever at least one node is lit, even if the master pass is off.
    if (this.glowMeshes.size === 0) {
      return;
    }
    this.ensureGlowTargets(width, height);
    const read = this.glowRead;
    const write = this.glowWrite;
    if (read == null || write == null) {
      return;
    }

    for (const [id, mesh] of this.glowMeshes) {
      const entry = this.entries[id];
      if (entry == null) {
        continue;
      }
      mesh.matrixWorld.copy(entry.mesh.matrixWorld);
      mesh.visible = entry.mesh.visible;
      const material = mesh.material as THREE.MeshBasicMaterial;
      material.color.copy(entry.material.color);
      material.opacity = entry.material.opacity;
    }

    this.renderer.setClearColor(0x000000, 1);
    this.renderer.setScissorTest(false);
    this.renderer.setRenderTarget(read);
    this.renderer.clear(true, false, false);
    this.renderer.render(this.glowScene, this.camera);

    this.glowQuad.material = this.glowBlurMaterial;
    this.glowBlurMaterial.uniforms.source.value = read.texture;
    (this.glowBlurMaterial.uniforms.direction.value as THREE.Vector2).set(
      1 / this.glowWidth,
      0,
    );
    this.renderer.setRenderTarget(write);
    this.renderer.clear(true, false, false);
    this.renderer.render(this.glowQuadScene, this.glowQuadCamera);

    this.glowBlurMaterial.uniforms.source.value = write.texture;
    (this.glowBlurMaterial.uniforms.direction.value as THREE.Vector2).set(
      0,
      1 / this.glowHeight,
    );
    this.renderer.setRenderTarget(read);
    this.renderer.clear(true, false, false);
    this.renderer.render(this.glowQuadScene, this.glowQuadCamera);

    let composite = this.glowCompositeMaterial;
    if (composite == null) {
      composite = new THREE.MeshBasicMaterial({
        transparent: true,
        blending: THREE.AdditiveBlending,
        depthTest: false,
        depthWrite: false,
        opacity: GLOW_OPACITY,
        toneMapped: false,
      });
      this.glowCompositeMaterial = composite;
    }
    if (composite.map !== read.texture) {
      const firstMap = composite.map == null;
      composite.map = read.texture;
      if (firstMap) {
        composite.needsUpdate = true;
      }
    }

    this.renderer.setRenderTarget(null);
    this.renderer.setViewport(0, 0, width, height);
    this.renderer.setScissorTest(false);
    this.renderer.clearDepth();
    this.glowQuad.material = composite;
    this.renderer.render(this.glowQuadScene, this.glowQuadCamera);
    this.renderer.setClearColor(0x000000, 0);
  }

  private disposeGlow(): void {
    this.clearGlowMeshes();
    this.glowRead?.dispose();
    this.glowWrite?.dispose();
    this.glowRead = null;
    this.glowWrite = null;
    this.glowCompositeMaterial?.dispose();
    this.glowCompositeMaterial = null;
    this.glowBlurMaterial.dispose();
    this.glowQuadGeometry.dispose();
    this.glowQuadScene.clear();
  }

  // -------------------------------------------------------------- axis gizmo

  private get gizmoSize(): number {
    const shortest = Math.min(this.host.clientWidth, this.host.clientHeight);
    return Math.max(76, Math.min(128, Math.floor(shortest * 0.28)));
  }

  private buildGizmo(): void {
    const backdrop = new THREE.Sprite(
      new THREE.SpriteMaterial({
        map: this.createBackdropTexture(),
        transparent: true,
        depthTest: false,
        depthWrite: false,
      }),
    );
    backdrop.scale.setScalar(3);
    backdrop.renderOrder = -1;
    this.gizmoScene.add(backdrop);
    this.gizmoResources.push(backdrop.material);

    const ballGeometry = new THREE.SphereGeometry(0.19, 20, 14);
    this.gizmoResources.push(ballGeometry);

    const pairs: Array<[ViewAxis, ViewAxis]> = [
      ['x', '-x'],
      ['y', '-y'],
      ['z', '-z'],
    ];
    for (const [positive, negative] of pairs) {
      const geometry = new THREE.BufferGeometry().setFromPoints([
        AXIS_VECTORS[negative].clone().multiplyScalar(1.02),
        AXIS_VECTORS[positive].clone().multiplyScalar(1.02),
      ]);
      const material = new THREE.LineBasicMaterial({
        color: AXIS_COLORS[positive],
        transparent: true,
        opacity: 0.38,
        depthWrite: false,
      });
      this.gizmoScene.add(new THREE.Line(geometry, material));
      this.gizmoResources.push(geometry, material);
    }

    for (const axis of AXIS_ORDER) {
      const negative = axis.startsWith('-');
      const material = new THREE.MeshBasicMaterial({
        color: AXIS_COLORS[axis],
        transparent: negative,
        opacity: negative ? 0.55 : 1,
        depthWrite: !negative,
      });
      const ball = new THREE.Mesh(ballGeometry, material);
      ball.position.copy(AXIS_VECTORS[axis]);
      this.gizmoScene.add(ball);
      this.gizmoResources.push(material);
      this.gizmoBalls.set(axis, ball);

      const label = new THREE.Sprite(
        new THREE.SpriteMaterial({
          map: this.createLabelTexture(AXIS_LABELS[axis]),
          transparent: true,
          depthTest: false,
          depthWrite: false,
        }),
      );
      label.position.copy(AXIS_VECTORS[axis]);
      label.scale.setScalar(0.5);
      label.renderOrder = 2;
      this.gizmoScene.add(label);
      this.gizmoResources.push(label.material);
      this.gizmoLabels.set(axis, label);
    }

    const ring = new THREE.Sprite(
      new THREE.SpriteMaterial({
        map: this.createRingTexture(),
        transparent: true,
        depthTest: false,
        depthWrite: false,
      }),
    );
    ring.scale.setScalar(0.66);
    ring.visible = false;
    ring.renderOrder = 3;
    this.gizmoScene.add(ring);
    this.gizmoResources.push(ring.material);
    this.gizmoSelection = ring;
    this.updateGizmoSelection();
  }

  private updateGizmoSelection(): void {
    for (const axis of AXIS_ORDER) {
      const ball = this.gizmoBalls.get(axis);
      if (ball != null) {
        ball.scale.setScalar(this.selectedAxis === axis ? 1.3 : 1);
      }
    }
    const ring = this.gizmoSelection;
    if (ring == null) {
      return;
    }
    if (this.selectedAxis == null) {
      ring.visible = false;
      return;
    }
    ring.visible = true;
    ring.position.copy(AXIS_VECTORS[this.selectedAxis]);
    (ring.material as THREE.SpriteMaterial).color.setHex(
      this.rotationLocked ? 0xe6b45c : 0x7fe8ff,
    );
  }

  private renderGizmo(width: number, height: number): void {
    const size = this.gizmoSize;
    const margin = this.gizmoMargin;
    this.syncGizmoCamera();

    this.renderer.setViewport(margin, height - size - margin, size, size);
    this.renderer.setScissor(margin, height - size - margin, size, size);
    this.renderer.setScissorTest(true);
    this.renderer.clearDepth();
    this.renderer.render(this.gizmoScene, this.gizmoCamera);
    this.renderer.setScissorTest(false);
    this.renderer.setViewport(0, 0, width, height);
  }

  /** Keep the gizmo camera aligned with the main view direction. */
  private syncGizmoCamera(): void {
    const direction = this.gizmoDirection.copy(this.camera.position).sub(this.controls.target);
    if (direction.lengthSq() < 1e-6) {
      direction.set(0, 0, 1);
    }
    direction.normalize().multiplyScalar(6);
    this.gizmoCamera.position.copy(direction);
    this.gizmoCamera.quaternion.copy(this.camera.quaternion);
    this.gizmoCamera.updateMatrixWorld();
  }

  private localPoint(event: PointerEvent): { x: number; y: number } {
    const rect = this.host.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  private inGizmo(point: { x: number; y: number }): boolean {
    const size = this.gizmoSize;
    const margin = this.gizmoMargin;
    return (
      point.x >= margin &&
      point.x <= margin + size &&
      point.y >= margin &&
      point.y <= margin + size
    );
  }

  private pickGizmoAxis(point: { x: number; y: number }): ViewAxis | null {
    const size = this.gizmoSize;
    const centerX = this.gizmoMargin + size * 0.5;
    const centerY = this.gizmoMargin + size * 0.5;
    const threshold = Math.max(16, size * 0.2);
    this.syncGizmoCamera();

    let best: ViewAxis | null = null;
    let bestDistance = Infinity;
    let bestDepth = -Infinity;

    for (const axis of AXIS_ORDER) {
      this.gizmoProjection.copy(AXIS_VECTORS[axis]);
      this.gizmoProjection.applyMatrix4(this.gizmoCamera.matrixWorldInverse);
      const depth = this.gizmoProjection.z;
      this.gizmoProjection.applyMatrix4(this.gizmoCamera.projectionMatrix);
      const px = centerX + this.gizmoProjection.x * size * 0.5;
      const py = centerY - this.gizmoProjection.y * size * 0.5;
      const distance = Math.hypot(px - point.x, py - point.y);
      if (distance > threshold) {
        continue;
      }
      // Overlapping axes: prefer the one closer to the viewer (larger view z).
      if (
        best == null ||
        distance < bestDistance - 1 ||
        (distance < bestDistance + 1 && depth > bestDepth)
      ) {
        best = axis;
        bestDistance = distance;
        bestDepth = depth;
      }
    }
    return best;
  }

  private createBackdropTexture(): THREE.Texture {
    const canvas = document.createElement('canvas');
    canvas.width = 128;
    canvas.height = 128;
    const context = canvas.getContext('2d');
    if (context) {
      const gradient = context.createRadialGradient(64, 64, 18, 64, 64, 62);
      gradient.addColorStop(0, 'rgba(10, 15, 26, 0.66)');
      gradient.addColorStop(0.72, 'rgba(10, 15, 26, 0.44)');
      gradient.addColorStop(1, 'rgba(10, 15, 26, 0)');
      context.fillStyle = gradient;
      context.beginPath();
      context.arc(64, 64, 62, 0, Math.PI * 2);
      context.fill();
      context.strokeStyle = 'rgba(148, 163, 184, 0.22)';
      context.lineWidth = 2;
      context.beginPath();
      context.arc(64, 64, 54, 0, Math.PI * 2);
      context.stroke();
    }
    const texture = new THREE.CanvasTexture(canvas);
    texture.colorSpace = THREE.SRGBColorSpace;
    this.gizmoTextures.push(texture);
    return texture;
  }

  private createLabelTexture(text: string): THREE.Texture {
    const canvas = document.createElement('canvas');
    canvas.width = 72;
    canvas.height = 72;
    const context = canvas.getContext('2d');
    if (context) {
      context.textAlign = 'center';
      context.textBaseline = 'middle';
      context.font = '700 30px "Inter", "Segoe UI", system-ui, sans-serif';
      context.lineWidth = 5;
      context.strokeStyle = 'rgba(6, 10, 18, 0.78)';
      context.strokeText(text, 36, 38);
      context.fillStyle = '#f4f9ff';
      context.fillText(text, 36, 38);
    }
    const texture = new THREE.CanvasTexture(canvas);
    texture.colorSpace = THREE.SRGBColorSpace;
    this.gizmoTextures.push(texture);
    return texture;
  }

  private createRingTexture(): THREE.Texture {
    const canvas = document.createElement('canvas');
    canvas.width = 128;
    canvas.height = 128;
    const context = canvas.getContext('2d');
    if (context) {
      context.strokeStyle = 'rgba(255, 255, 255, 0.92)';
      context.lineWidth = 9;
      context.beginPath();
      context.arc(64, 64, 50, 0, Math.PI * 2);
      context.stroke();
    }
    const texture = new THREE.CanvasTexture(canvas);
    texture.colorSpace = THREE.SRGBColorSpace;
    this.gizmoTextures.push(texture);
    return texture;
  }

  private disposeGizmo(): void {
    for (const resource of this.gizmoResources) {
      resource.dispose();
    }
    this.gizmoResources.length = 0;
    this.gizmoScene.clear();
    for (const texture of this.gizmoTextures) {
      texture.dispose();
    }
    this.gizmoTextures.length = 0;
    this.gizmoBalls.clear();
    this.gizmoLabels.clear();
    this.gizmoSelection = null;
  }
}
