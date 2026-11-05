import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import type { Model, MeshModel, GradientModel } from '../../sdk/index.mjs';

/** 六个 Blender 风格观察方向：+X、-X、+Y、-Y、+Z、-Z。 */
export type ViewAxis = 'x' | '-x' | 'y' | '-y' | 'z' | '-z';

/** Stage 向外抛的回调；目前只有点击 gizmo 轴向时触发。 */
export interface StageCallbacks {
  /** 用户点击 gizmo 的某个轴后回调（参数为对应的视角方向）。 */
  onAxisPick?: (axis: ViewAxis) => void;
}

/** 暴露给 UI / 外部 API 的可选节点（与 SDK 的 NodeModel 同构，按渲染顺序 id 排列）。 */
export interface StageNode {
  id: number;
  name: string;
  kind: number;
  animated: boolean;
  vertexCount: number;
  indexCount: number;
}

/** 轴向在 gizmo 与提示文案中显示的短标签。 */
export const AXIS_LABELS: Record<ViewAxis, string> = {
  x: '+X',
  '-x': '-X',
  y: '+Y',
  '-y': '-Y',
  z: '+Z',
  '-z': '-Z',
};

/** gizmo 的固定绘制 / 拾取顺序（正轴在前，负轴在后）。 */
const AXIS_ORDER: ViewAxis[] = ['x', '-x', 'y', '-y', 'z', '-z'];

const AXIS_VECTORS: Record<ViewAxis, THREE.Vector3> = {
  x: new THREE.Vector3(1, 0, 0),
  '-x': new THREE.Vector3(-1, 0, 0),
  y: new THREE.Vector3(0, 1, 0),
  '-y': new THREE.Vector3(0, -1, 0),
  z: new THREE.Vector3(0, 0, 1),
  '-z': new THREE.Vector3(0, 0, -1),
};

/** 每个轴向的 gizmo 颜色：正轴亮、负轴暗，便于区分朝向。 */
const AXIS_COLORS: Record<ViewAxis, number> = {
  x: 0xff6b6b,
  '-x': 0x9c4a52,
  y: 0x7dff9e,
  '-y': 0x45875a,
  z: 0x6ea8ff,
  '-z': 0x466a9f,
};

/** 默认视角：正对 +Z（Blender 的正视图），相机距离原点 7.2。 */
const AXIS_HOME = new THREE.Vector3(0, 0, 7.2);
/** 轴向切换的补间时长（毫秒）。 */
const VIEW_TWEEN_MS = 320;

/** 选中线框颜色（蓝色）。 */
const SELECTION_COLOR = 0x3d8bff;
/**
 * 节点重排时的层深微调步进（世界单位）。
 * 层深相同的填充/描边子节点靠它获得严格的前后次序，避免同级共面闪烁。
 */
const STACK_EPSILON = 0.002;
/** 柔光渲染目标相对主画布的缩放：1/4 分辨率即可，模糊后叠加。 */
const GLOW_SCALE = 0.25;
/** 柔光合成基础透明度；实际叠加值再乘以强度倍率。 */
const GLOW_OPACITY = 0.8;

/** 全屏四边形顶点着色器：直接把裁剪空间坐标铺满，只传 uv 给片元。 */
const FULLSCREEN_VERTEX = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

/**
 * 9 抽样一维高斯模糊（5 个权重位置，左右对称）。
 * 两遍分别沿 X / Y 方向执行，即可近似二维高斯；direction 由 JS 每遍设置。
 */
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

/** 渐变 LUT 的采样点数；一条渐变轴（往往只有几像素宽）远达不到这个密度。 */
const GRADIENT_LUT_SIZE = 256;

/** 把数值钳制到 0..1。 */
function clamp01(value: number): number {
  return value < 0 ? 0 : value > 1 ? 1 : value;
}

/**
 * 把渐变 stop 烘焙成 `GRADIENT_LUT_SIZE × 1` 的 RGBA8 查找纹理。
 *
 * RGB 按 sRGB 原值（字节）存放，纹理**不**标记为 sRGB：这样 GPU 的线性
 * 过滤是在 sRGB 编码空间里插值，与 Lottie / lottie-web 对 stop 的插值
 * 语义一致；片元着色器采样后再手动解码到线性空间参与光照。
 * 若直接把纹理标成 `SRGBColorSpace`，GPU 会先逐纹素解码再做线性插值，
 * 中间色会明显偏亮、偏饱和。
 *
 * alpha 是线性值，不参与 sRGB 解码。
 */
function buildGradientLut(gradient: GradientModel): {
  lut: THREE.DataTexture;
  variesAlpha: boolean;
} {
  const pixels = new Uint8Array(GRADIENT_LUT_SIZE * 4);
  const stops = gradient.stops;
  const alphaStops = gradient.alphaStops;
  let variesAlpha = false;
  for (let i = 0; i < GRADIENT_LUT_SIZE; i += 1) {
    const t = i / (GRADIENT_LUT_SIZE - 1);
    // 颜色插值：落在首个 stop 之前取首色，落在末个之后取末色。
    let rgb: [number, number, number] = [1, 1, 1];
    if (stops.length > 0) {
      rgb = stops[0].color;
      for (let stop = 1; stop < stops.length; stop += 1) {
        const previous = stops[stop - 1];
        const next = stops[stop];
        if (t <= next.offset) {
          const span = Math.max(1e-6, next.offset - previous.offset);
          const k = clamp01((t - previous.offset) / span);
          rgb = [
            previous.color[0] + (next.color[0] - previous.color[0]) * k,
            previous.color[1] + (next.color[1] - previous.color[1]) * k,
            previous.color[2] + (next.color[2] - previous.color[2]) * k,
          ];
          break;
        }
        rgb = next.color;
      }
    }
    // 透明度插值；无 stop 时整段不透明。
    let alpha = 1;
    if (alphaStops.length > 0) {
      alpha = alphaStops[0].alpha;
      for (let stop = 1; stop < alphaStops.length; stop += 1) {
        const previous = alphaStops[stop - 1];
        const next = alphaStops[stop];
        if (t <= next.offset) {
          const span = Math.max(1e-6, next.offset - previous.offset);
          alpha = previous.alpha + (next.alpha - previous.alpha) * clamp01((t - previous.offset) / span);
          break;
        }
        alpha = next.alpha;
      }
    }
    if (alpha < 0.999) {
      variesAlpha = true;
    }
    pixels[i * 4] = Math.round(clamp01(rgb[0]) * 255);
    pixels[i * 4 + 1] = Math.round(clamp01(rgb[1]) * 255);
    pixels[i * 4 + 2] = Math.round(clamp01(rgb[2]) * 255);
    pixels[i * 4 + 3] = Math.round(clamp01(alpha) * 255);
  }
  const lut = new THREE.DataTexture(
    pixels,
    GRADIENT_LUT_SIZE,
    1,
    THREE.RGBAFormat,
    THREE.UnsignedByteType,
  );
  lut.colorSpace = THREE.NoColorSpace;
  // 只需沿 t 轴线性过滤；纹理是常量，不需要 mipmap / 重复。
  // 注意：过滤发生在 sRGB 编码空间（见函数注释），解码在片元里做。
  lut.minFilter = THREE.LinearFilter;
  lut.magFilter = THREE.LinearFilter;
  lut.wrapS = THREE.ClampToEdgeWrapping;
  lut.wrapT = THREE.ClampToEdgeWrapping;
  lut.generateMipmaps = false;
  lut.needsUpdate = true;
  return { lut, variesAlpha };
}

/**
 * 给 MeshLambertMaterial 注入逐片元渐变求值。
 *
 * 顶点着色器把网格局部坐标（`transformed.xy`，与渐变端点同一坐标系）传给
 * 片元；片元按线性 / 径向公式算出 `t`，采样 1D LUT，再乘进 `diffuseColor`：
 * RGB 参与光照，alpha 与材质透明度（图层不透明度）相乘。
 */
function applyGradientShader(
  material: THREE.Material,
  gradient: GradientModel,
  lut: THREE.DataTexture,
): void {
  const uniforms = {
    uGradLut: { value: lut },
    uGradStart: { value: new THREE.Vector2(gradient.start[0], gradient.start[1]) },
    uGradEnd: { value: new THREE.Vector2(gradient.end[0], gradient.end[1]) },
    uGradKind: { value: gradient.kind },
  };
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, uniforms);
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', '#include <common>\nvarying vec2 vGradPos;')
      .replace('#include <begin_vertex>', '#include <begin_vertex>\n  vGradPos = transformed.xy;');
    shader.fragmentShader = shader.fragmentShader
      .replace(
        '#include <common>',
        `#include <common>
uniform sampler2D uGradLut;
uniform vec2 uGradStart;
uniform vec2 uGradEnd;
uniform float uGradKind;
varying vec2 vGradPos;`,
      )
      // 在颜色阶段注入：此时 diffuseColor 尚参与光照，乘上去与顶点色等价。
      .replace(
        '#include <color_fragment>',
        `#include <color_fragment>
  float gradT;
  if (uGradKind < 1.5) {
    vec2 axis = uGradEnd - uGradStart;
    gradT = dot(vGradPos - uGradStart, axis) / max(dot(axis, axis), 1e-6);
  } else {
    gradT = length(vGradPos - uGradStart) / max(length(uGradEnd - uGradStart), 1e-6);
  }
  vec4 gradSample = texture2D(uGradLut, vec2(clamp(gradT, 0.0, 1.0), 0.5));
  // LUT 里存的是 sRGB 编码值（插值也在该空间完成），这里逐像素解码到
  // 线性空间再参与光照，等价于 Lottie 的 sRGB 渐变。
  vec3 gradRgb = mix(
    gradSample.rgb / 12.92,
    pow((gradSample.rgb + 0.055) / 1.055, vec3(2.4)),
    step(vec3(0.04045), gradSample.rgb)
  );
  diffuseColor.rgb *= gradRgb;
  diffuseColor.a *= gradSample.a;`,
      );
  };
  // 所有渐变材质共用同一份注入代码，显式固定缓存键，便于复用已编译程序。
  material.customProgramCacheKey = () => 'l2d-gradient';
}

/** 一个网格对应的全部运行时对象；数组下标即节点 id。 */
interface Entry {
  /** SDK 的网格模型（只读数据源）。 */
  data: MeshModel;
  mesh: THREE.Mesh;
  geometry: THREE.BufferGeometry;
  material: THREE.MeshLambertMaterial | THREE.MeshBasicMaterial;
  /** 是否为图片贴图网格（kind = 2）；贴图加载完成前不绘制。 */
  textured: boolean;
  /** 贴图是否已加载完成（缺失 / 失败时保持 false）。 */
  textureReady: boolean;
  /** 贴图加载失败（404 / 解码失败）；之后保持隐藏。 */
  textureFailed: boolean;
  /** 静态变换层的固定基矩阵（第 0 帧拷贝，避免每帧从轨道读取）。 */
  base: THREE.Matrix4;
  /** 逐帧变换轨道（frameCount × 16，列主序）；无轨道时为 null。 */
  track: Float32Array | null;
  /** 该轨道是否全程不变（只上传一次矩阵）。 */
  transformStatic: boolean;
  /** 逐帧颜色轨道（frameCount × 4）或 null。 */
  colors: Float32Array | null;
  /** 顶点属性（动画网格为 DynamicDrawUsage）。 */
  position: THREE.BufferAttribute;
  /** 是否需要逐帧拷贝顶点。 */
  positionsAnimated: boolean;
  /** 每个拓扑桶一份索引属性（至少一项；0 与静态 indices 相同）。 */
  indexAttributes: THREE.BufferAttribute[];
  /**
   * 渐变的一维查找纹理（256×1 RGBA8），非渐变网格为 null。
   * 颜色在片元着色器里按局部坐标逐像素求值，与顶点密度无关。
   */
  gradientLut: THREE.DataTexture | null;
  /** 当前绑定到 geometry 的桶下标，用于跳过重复 setIndex。 */
  currentBucket: number;
  /** 由基础 alpha 推导的可见性（尚未叠加手动隐藏）。 */
  staticVisible: boolean;
  /**
   * 节点重排产生的额外层深偏移（世界单位）。
   * 目标深度 = 原始烘焙深度 + stackDelta，因此“层深”滑块仍可整体缩放。
   */
  stackDelta: number;
  /** 同层共面网格的绘制层序偏移（世界单位，见 setModel 里的说明）。 */
  stackOffset: number;
}

/**
 * 判断一条变换轨道是否整段不变（逐帧比较 16 个矩阵元素）。
 * 常量轨道只需上传一次矩阵，播放时完全跳过矩阵写入。
 */
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
 * 统计相邻图层深度之间的最小正间距（世界单位）。
 *
 * `model.meshes` 已按 `renderOrder`（即深度）升序排列，因此只需比较相邻项；
 * 同一图层内的网格深度相同（差值为 0）会被跳过。没有两层不同深度时返回 0，
 * 调用方据此关闭共面偏移。子合成内部的深度差（±0.01 量级）也会被计入，
 * 因此得到的值足以保护跨层遮挡。
 */
function measureMinLayerGap(meshes: readonly MeshModel[]): number {
  let gap = Number.POSITIVE_INFINITY;
  for (let i = 1; i < meshes.length; i += 1) {
    const delta = meshes[i].depth - meshes[i - 1].depth;
    if (delta > 1e-5 && delta < gap) {
      gap = delta;
    }
  }
  return gap === Number.POSITIVE_INFINITY ? 0 : gap;
}

/**
 * 基于 Rust 编译结果的 three.js 舞台。
 *
 * 关键设计：每个网格只创建一次固定的 BufferGeometry；逐帧只做两件事——
 * 1. 动画网格：把烘焙好的顶点切片拷贝进 position 数组并标记 needsUpdate；
 * 2. 所有网格：直接把世界矩阵写进 `mesh.matrixWorld`（关闭自动更新，
 *    图层都挂在场景根下，因此无需父级乘法，也不会触发矩阵失效）。
 * 其余（三角化、关键帧求值、深度烘焙）全部在转换阶段完成。
 *
 * 此外还负责：节点选择线框、逐节点显隐、柔光后处理、轴向 gizmo。
 */
export class Stage {
  // ---- 渲染基础设施 ----
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera: THREE.PerspectiveCamera;
  private readonly controls: OrbitControls;
  private readonly host: HTMLElement;

  // ---- 模型与逐帧状态 ----
  /** 已加载的所有网格运行时对象，下标 = 节点 id。 */
  private readonly entries: Entry[] = [];
  /** 当前模型；为 null 表示尚未加载。 */
  private model: Model | null = null;
  /** 已应用的帧下标；-1 表示还没写过任何帧。 */
  private frameIndex = -1;
  /** 脏标记：仅相机移动或有数据改动时才真正重绘（脏渲染）。 */
  private dirty = true;
  /** “层深”滑块倍率；实际只给矩阵 Z 平移追加增量。 */
  private depthAmount = 1;
  /** “厚度”滑块倍率；实际等价于矩阵后乘 scale(1, 1, thickness)。 */
  private thicknessAmount = 1;
  /** 渲染像素比；低帧时会自适应下调。 */
  private pixelRatio = 1.5;
  /** 连续低帧窗口计数，达到阈值才降一次分辨率。 */
  private lowFpsWindows = 0;
  private readonly observer: ResizeObserver;
  private readonly callbacks: StageCallbacks;
  /** 对外暴露的节点列表（渲染顺序）。 */
  private readonly nodes: StageNode[] = [];

  // ---- 选中线框 ----
  /** 每个被选节点一条蓝色 1px 包围盒线框（Box3Helper）。 */
  private readonly selectionBoxes = new Map<number, THREE.Box3Helper>();
  /** 复用的临时包围盒，避免每次更新都分配 Three 对象。 */
  private readonly boxScratch = new THREE.Box3();

  // ---- 透视补偿 ----
  /** 上一帧相机的逆矩阵（每帧重算一次，所有图层共用）。 */
  private readonly cameraInverse = new THREE.Matrix4();
  /** 相机到观察目标的距离，即补偿后的投影参考平面。 */
  private referenceDistance = 1;
  /** 复用向量：把图层原点换到相机空间求深度，避免逐层分配。 */
  private readonly cameraSpacePoint = new THREE.Vector3();
  /** 复用矩阵：拼装相机空间的补偿变换。 */
  private readonly matrixScratch = new THREE.Matrix4();
  /** 舞台点选复用的射线；避免每次点击分配新对象。 */
  private readonly raycaster = new THREE.Raycaster();

  /** 通过节点列表隐藏的节点 id；隐藏后跳过逐帧计算与绘制。 */
  private readonly hiddenNodes = new Set<number>();

  // ---- 柔光后处理 ----
  /** 柔光强度倍率（0..2）：合成叠加的不透明度 = 基础值 × 该倍率。 */
  private glowIntensity = 1;
  /** 柔光半径倍率（0.2..3）：高斯模糊的采样步长按它缩放。 */
  private glowRadius = 1;

  /** 总柔光开关：开启时默认所有节点都发光，关闭时仅显式开启的节点发光。 */
  private softGlow = false;
  /** 单节点覆盖标记（显式开 / 关），与总开关组合出最终语义。 */
  private readonly glowFlags = new Map<number, boolean>();
  /** 参与柔光的节点 → 借用主网格 geometry 的“外壳”网格（只换纯色材质）。 */
  private readonly glowMeshes = new Map<number, THREE.Mesh>();
  /** 独立的柔光场景：只渲染发光外壳，避免主场景被重复遍历。 */
  private readonly glowScene = new THREE.Scene();
  /** 双缓冲渲染目标：读 → 写做横向模糊，再写回读做纵向模糊。 */
  private glowRead: THREE.WebGLRenderTarget | null = null;
  private glowWrite: THREE.WebGLRenderTarget | null = null;
  /** 当前柔光目标的像素尺寸（变化时重建 RT）。 */
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

  // ---- 左上角 Blender 风格朝向 gizmo ----
  /** gizmo 用独立的正交相机 + 小场景，以视口（scissor）叠加在主画面左上角。 */
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
  /** 按下时的指针位置（用于区分“点击”与拖拽）；null 表示未在 gizmo 内按下。 */
  private gizmoPointerDown: { x: number; y: number } | null = null;
  /** gizmo 距容器左上角的边距（CSS px）。 */
  private readonly gizmoMargin = 12;
  /** 复用向量：保存相机相对目标的方向，避免每帧分配。 */
  private readonly gizmoDirection = new THREE.Vector3();

  /**
   * 捕获阶段监听 pointerdown：只有点在 gizmo 区域内才接管事件，
   * 从而在 OrbitControls 的拖拽逻辑之前处理“点击轴向”。
   */
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

  /** 抬起时若位移小于 6px 视为点击，再做轴向拾取；拖动超过该距离则忽略。 */
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

  /**
   * @param host 承载 WebGL canvas 的容器（尺寸变化由 ResizeObserver 自动跟随）
   * @param callbacks 可选回调（例如 gizmo 轴向点击）
   */
  constructor(host: HTMLElement, callbacks: StageCallbacks = {}) {
    this.host = host;
    this.callbacks = callbacks;
    // alpha + 透明清屏色：让页面背景的渐变透出来。
    this.renderer = new THREE.WebGLRenderer({
      antialias: true,
      alpha: true,
      powerPreference: 'high-performance',
      // 对数深度缓冲：同一图层的多个 drawable 挤出面完全共面，只能靠极小的 Z
      // 偏移还原 Lottie 的绘制顺序；线性深度在 z≈7 处每个 16 位深度单位约合
      // 0.0037 世界单位，这类百分之一量级的偏移会被量化成 z-fighting 虚线
      // （dalek 拔塞器蓝色内圈因此被切成斜虚线）。对数深度按相对量均匀分配
      // 精度，同一偏移可得到成百上千个深度单位。
      logarithmicDepthBuffer: true,
    });
    // 限制 DPR，避免高分屏 4K 画布带来 4 倍以上填充率开销。
    this.pixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
    this.renderer.setPixelRatio(this.pixelRatio);
    this.renderer.setClearColor(0x000000, 0);
    // 手动清屏：主画面、柔光、gizmo 分多趟绘制。
    this.renderer.autoClear = false;
    this.renderer.domElement.className = 'three-canvas';
    host.appendChild(this.renderer.domElement);

    // 40° 透视，默认站在 +Z 方向（AXIS_HOME）。
    this.camera = new THREE.PerspectiveCamera(40, 1, 0.1, 60);
    this.camera.position.copy(AXIS_HOME);

    // 环绕控制：阻尼 + 距离限制，关闭自动旋转（由 UI 开关控制）。
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

    // 固定三点布光 + 环境光：突出挤出薄板的体积感与正面颜色。
    const ambient = new THREE.AmbientLight(0xffffff, 1.25);
    const key = new THREE.DirectionalLight(0xdfe9ff, 2.4);
    key.position.set(2.4, 3.2, 4.2);
    const rim = new THREE.DirectionalLight(0xffb98a, 1.6);
    rim.position.set(-3.4, -1.6, -2.2);
    const fill = new THREE.DirectionalLight(0x86a8ff, 0.9);
    fill.position.set(-1.5, -2.8, 3.4);
    this.scene.add(ambient, key, rim, fill);

    this.buildGizmo();
    // 全屏四边形始终充满视口，不能被视锥剔除。
    this.glowQuad.frustumCulled = false;
    this.glowQuadScene.add(this.glowQuad);
    // 在容器上用**捕获阶段**监听 pointerdown/up，确保 gizmo 的点击早于
    // OrbitControls 画布监听器启动拖拽，避免点击轴向时同时触发旋转。
    host.addEventListener('pointerdown', this.onGizmoPointerDown, true);
    host.addEventListener('pointerup', this.onGizmoPointerUp, true);

    // 容器尺寸变化时同步 renderer / camera 纵横比。
    this.observer = new ResizeObserver(() => this.resize());
    this.observer.observe(host);
    this.resize();
  }

  get hostElement(): HTMLElement {
    return this.host;
  }

  /**
   * 载入新模型：为每个网格建立固定的 geometry / material / 索引桶，
   * 初始化节点列表并立即应用第 0 帧。
   * 同一模型的网格按 renderOrder 排序，因此这里的下标就是节点 id。
   */
  setModel(model: Model): void {
    this.clear();
    this.model = model;
    this.frameIndex = -1;
    this.nodes.length = 0;
    this.glowFlags.clear();

    // 同一图层的多个 drawable 共享一条变换轨道；常量轨道检测按轨道去重，
    // 避免对几千条相同轨道重复逐帧比较。
    const staticTracks = new Map<number, boolean>();

    // 同一图层的所有 drawable 共用一条变换轨道，因此它们的挤出面完全共面；
    // Lottie 靠绘制顺序决定上下，3D 里只能把该顺序转成极小的 Z 偏移，否则
    // 共面片之间的深度差会把下层颜色从缝隙里切出一道虚线。偏移必须限制在
    // “相邻图层最小层距”的一半以内，否则本图层最后几个 drawable 会跨到
    // 邻层前面，破坏跨层遮挡。
    const minLayerGap = measureMinLayerGap(model.meshes);
    // 每个图层（transformIndex）包含的 drawable 数量：决定该层的偏移步长。
    const stackTotals = new Map<number, number>();
    for (const data of model.meshes) {
      stackTotals.set(data.transformIndex, (stackTotals.get(data.transformIndex) ?? 0) + 1);
    }
    const stackCounts = new Map<number, number>();

    for (const [id, data] of model.meshes.entries()) {
      const stackIndex = stackCounts.get(data.transformIndex) ?? 0;
      stackCounts.set(data.transformIndex, stackIndex + 1);
      // 该层共面网格的 Z 步长：整层偏移带宽不超过层距的一半（且不超过
      // STACK_EPSILON）；数量只有 1 的图层无需偏移。
      const stackTotal = stackTotals.get(data.transformIndex) ?? 1;
      const stackStep =
        stackTotal > 1
          ? Math.min(STACK_EPSILON, (minLayerGap * 0.45) / (stackTotal - 1))
          : 0;
      // 动画网格必须独占一份顶点缓冲（会逐帧覆盖）；静态网格可直接引用 SDK 的数组。
      const positionsAnimated = data.animated && data.framePositions != null;
      const positions = positionsAnimated
        ? new Float32Array(data.positions)
        : data.positions;

      // 每个网格只建一次 geometry；顶点数取“跨桶最大值”，因此任何桶都能复用。
      const geometry = new THREE.BufferGeometry();
      const position = new THREE.BufferAttribute(positions, 3);
      position.setUsage(positionsAnimated ? THREE.DynamicDrawUsage : THREE.StaticDrawUsage);
      geometry.setAttribute('position', position);
      // 渐变网格（v6）：颜色在片元着色器里按局部坐标逐像素求值，与顶点
      // 密度无关。这里把 stop 烘焙成 1D LUT；`variesAlpha` 决定是否需要
      // 透明混合（渐变 alpha 低于 1 时必须开启）。
      let gradientLut: THREE.DataTexture | null = null;
      let gradientAlpha = false;
      if (data.gradient) {
        const built = buildGradientLut(data.gradient);
        gradientLut = built.lut;
        gradientAlpha = built.variesAlpha;
      }
      // 每个拓扑桶一份索引属性；默认绑定桶 0，逐帧按 frameBuckets 切换。
      const indexAttributes = data.buckets.map(
        (bucket) => new THREE.BufferAttribute(bucket.indices, 1),
      );
      geometry.setIndex(indexAttributes[0]);
      // 图片网格（v4）：携带逐顶点 UV，材质使用不受光照影响的贴图。
      const textured = data.texture != null && data.uvs != null;
      if (data.uvs != null) {
        geometry.setAttribute(
          'uv',
          new THREE.BufferAttribute(new Float32Array(data.uvs), 2),
        );
      }

      // 变换轨道：同层共享；track 为 null 时退化为单位矩阵（base 保持默认）。
      const track = model.transforms[data.transformIndex] ?? null;
      let transformStatic = staticTracks.get(data.transformIndex);
      if (transformStatic === undefined) {
        transformStatic = track == null || isConstantTrack(track, model.frameCount);
        staticTracks.set(data.transformIndex, transformStatic);
      }

      // 初始颜色 / 透明度：有逐帧颜色轨道时取第 0 帧，否则用静态颜色。
      const colors = data.colors;
      const alpha = colors ? colors[3] : data.color[3];
      // Lottie 颜色是 sRGB；这里显式按 SRGBColorSpace 构造颜色，让静态与动画颜色
      // 都进入 three.js 的线性空间，光照计算才一致。
      const baseColor = new THREE.Color().setRGB(
        data.color[0],
        data.color[1],
        data.color[2],
        THREE.SRGBColorSpace,
      );
      const material: THREE.MeshLambertMaterial | THREE.MeshBasicMaterial = textured
        ? // 图片贴图：颜色完全由纹理决定，不参与光照；alpha 仍来自图层透明度。
          new THREE.MeshBasicMaterial({
            color: 0xffffff,
            side: THREE.DoubleSide,
            transparent: true,
            opacity: Math.max(0, Math.min(1, alpha)),
          })
        : new THREE.MeshLambertMaterial({
            color: baseColor,
            // 渐变网格的颜色由片元注入，不再使用顶点色属性。
            vertexColors: false,
            // 平面着色：挤出薄板的每个面亮度独立，轮廓更“硬”。
            flatShading: true,
            side: THREE.DoubleSide,
            // 常量且不透明的颜色直接关闭透明混合，省掉排序与 overdraw。
            // 渐变 alpha 低于 1 时同样需要透明混合。
            transparent: colors != null || alpha < 0.999 || gradientAlpha,
            opacity: Math.max(0, Math.min(1, alpha)),
          });
      // 渐变网格：注入逐片元求值，颜色只与片元位置有关。
      if (data.gradient && gradientLut) {
        applyGradientShader(material, data.gradient, gradientLut);
      }
      // 关闭全部矩阵自动更新：世界矩阵由 applyFrame 直接写入。
      // 同时关闭视锥剔除，避免整批网格在相机边缘被跳过（模型尺寸已归一化）。
      const mesh = new THREE.Mesh(geometry, material);
      mesh.matrixAutoUpdate = false;
      mesh.matrixWorldAutoUpdate = false;
      mesh.matrixWorldNeedsUpdate = false;
      mesh.frustumCulled = false;
      mesh.renderOrder = 0;
      // alpha 低于 0.4% 视为完全不可见，直接不绘制。
      const staticVisible = alpha >= 0.004;
      // 图片网格要等纹理加载完成才显示，避免闪出一块白板。
      mesh.visible = textured ? false : staticVisible;
      this.scene.add(mesh);

      // 静态基矩阵：取轨道第 0 帧；无常量轨道时保持单位矩阵。
      const base = new THREE.Matrix4();
      if (track) {
        base.fromArray(track, 0);
      }

      this.entries.push({
        data,
        mesh,
        geometry,
        material,
        textured,
        textureReady: false,
        textureFailed: false,
        base,
        track,
        transformStatic,
        colors,
        position,
        positionsAnimated,
        indexAttributes,
        gradientLut,
        currentBucket: 0,
        staticVisible,
        // 新载入的模型按烘焙层深排序，尚未发生拖拽重排。
        stackDelta: 0,
        // 同层共面网格按绘制顺序微移：后画的（stackIndex 更大）更靠前。
        stackOffset: stackIndex * stackStep,
      });
      // 图片纹理异步加载；加载完成前保持隐藏。404 / 解码失败时永久隐藏，
      // 而不是显示一块白板（资源缺失时该图层优雅跳过）。
      if (textured) {
        const entry = this.entries[this.entries.length - 1];
        const texture = new THREE.TextureLoader().load(
          data.texture as string,
          (loaded) => {
            loaded.colorSpace = THREE.SRGBColorSpace;
            entry.textureReady = true;
            this.applyFrame(this.frameIndex);
            this.dirty = true;
          },
          undefined,
          () => {
            entry.textureFailed = true;
            entry.mesh.visible = false;
            this.dirty = true;
          },
        );
        texture.colorSpace = THREE.SRGBColorSpace;
        material.map = texture;
        material.needsUpdate = true;
      }
      this.nodes.push({
        id,
        name: data.name || `节点 ${id + 1}`,
        kind: data.kind,
        animated: data.animated,
        vertexCount: data.vertexCount,
        indexCount: data.indexCount,
      });
    }

    // 立即写入第 0 帧，保证加载完成即可见。
    this.applyFrame(0);
    this.syncGlowMeshes();
    this.dirty = true;
  }

  /** 返回当前层级顺序的节点列表；其 id 与 setSelection 等接口使用的 id 一致。 */
  getNodes(): readonly StageNode[] {
    return this.nodes;
  }

  /**
   * 拾取屏幕坐标下的节点（用于“舞台选取”模式）。
   *
   * 只对当前实际绘制的网格做射线求交：`mesh.visible` 已综合了逐帧 alpha、
   * 手动隐藏与贴图就绪状态，因此这里直接以它为准；多个网格重叠时取最近
   * 命中，保证选中的是视觉上最前面的节点。
   *
   * @param clientX 页面坐标（CSS px，与 PointerEvent.clientX 一致）
   * @param clientY 页面坐标（CSS px）
   * @returns 命中的节点 id；未命中返回 null
   */
  pickNodeAt(clientX: number, clientY: number): number | null {
    if (this.entries.length === 0) {
      return null;
    }
    const rect = this.host.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      return null;
    }
    // 页面坐标 → 容器内 NDC（-1..1，y 轴向上为正）。
    const ndc = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -(((clientY - rect.top) / rect.height) * 2 - 1),
    );
    // 相机矩阵在渲染循环里更新；点击发生在两帧之间，这里补一次，
    // 避免刚拖动完视角就点击时用到上一帧的矩阵。
    this.camera.updateMatrixWorld();
    this.raycaster.setFromCamera(ndc, this.camera);

    const meshes: THREE.Mesh[] = [];
    for (const entry of this.entries) {
      if (entry.mesh.visible) {
        // 动画网格的顶点逐帧变化，创建时算出的包围球可能已过期，
        // 过期包围球会让射线在有效像素上提前判定未命中。
        entry.geometry.computeBoundingSphere();
        meshes.push(entry.mesh);
      }
    }
    const hits = this.raycaster.intersectObjects(meshes, false);
    if (hits.length === 0) {
      return null;
    }
    const object = hits[0].object;
    const index = this.entries.findIndex((entry) => entry.mesh === object);
    return index >= 0 ? index : null;
  }

  /**
   * 按节点列表的新顺序重新安排前后层级。
   *
   * 节点 id 与几何缓冲保持稳定，只重排显示列表并重算层深：把原始层深
   * 升序排列后依次赋给新顺序的节点（列表自上而下对应由后到前），再用
   * [`STACK_EPSILON`] 打破同层深的平局。层深最终写入矩阵 Z 平移，因此
   * “层深”滑块仍可在此基础上整体缩放。
   *
   * 传入的 id 必须是全部节点的完整排列，缺项或重复时直接忽略。
   */
  setNodeOrder(ids: readonly number[]): void {
    if (ids.length !== this.entries.length || ids.length === 0) {
      return;
    }

    // 先校验并收集节点对象；byId 用完后为空才说明传入的是完整排列。
    const byId = new Map<number, StageNode>();
    for (const node of this.nodes) {
      byId.set(node.id, node);
    }
    const next: StageNode[] = [];
    for (const id of ids) {
      const node = byId.get(id);
      if (node == null) {
        return;
      }
      byId.delete(id);
      next.push(node);
    }
    if (byId.size > 0) {
      return;
    }

    this.nodes.length = 0;
    this.nodes.push(...next);

    // 原始层深升序：默认列表中越靠上深度越小（越靠后）。
    const depths = this.entries.map((entry) => entry.data.depth).sort((a, b) => a - b);
    for (let rank = 0; rank < ids.length; rank += 1) {
      const entry = this.entries[ids[rank]];
      const target = depths[rank] + rank * STACK_EPSILON;
      entry.stackDelta = target - entry.data.depth;
    }

    // 立即重放当前帧，让新层级即时生效。
    this.applyFrame(Math.max(0, this.frameIndex));
    this.dirty = true;
  }

  /**
   * 设置选中节点集合（内部会校验 id 范围）。
   * 只为新增的节点创建蓝色线框，移除的节点立即释放几何与材质，
   * 然后统一刷新包围盒。
   */
  setSelection(ids: readonly number[]): void {
    const wanted = new Set<number>();
    for (const id of ids) {
      if (id >= 0 && id < this.entries.length) {
        wanted.add(id);
      }
    }

    // 取消选中的节点：移出场景并释放线框资源，避免频繁多选时泄漏。
    for (const [id, helper] of this.selectionBoxes) {
      if (!wanted.has(id)) {
        this.scene.remove(helper);
        helper.geometry.dispose();
        (helper.material as THREE.Material).dispose();
        this.selectionBoxes.delete(id);
      }
    }

    // 新选中的节点：创建深度测试关闭的线框，保证被模型挡住时仍可见。
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

  /**
   * 切换总柔光。开启后默认所有节点参与柔光，单节点仍可用 setNodeGlow 排除；
   * 关闭后只有显式开启的节点继续发光（见 isNodeGlowEnabled）。
   */
  setSoftGlow(enabled: boolean): void {
    if (this.softGlow === enabled) {
      return;
    }
    this.softGlow = enabled;
    this.syncGlowMeshes();
    this.dirty = true;
  }

  /**
   * 单独开启 / 关闭某个节点的柔光（覆盖总开关的默认行为）。
   * @param id 节点 id（= entries 下标）
   * @param enabled 是否发光
   */
  setNodeGlow(id: number, enabled: boolean): void {
    if (this.glowFlags.get(id) === enabled) {
      return;
    }
    this.glowFlags.set(id, enabled);
    this.syncGlowMeshes();
    this.dirty = true;
  }

  /**
   * 把总开关的取值一次性写进所有节点的覆盖标记。
   * 这样“关总开关”时不会残留旧的 true，之后单节点仍可自行开启。
   */
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

  /**
   * 调整柔光强度（0..2，1 为默认）：只改合成叠加的不透明度，
   * 不重建渲染目标，暂停时也会立即重绘。
   */
  setGlowIntensity(value: number): void {
    const next = Math.max(0, Math.min(2, value));
    if (this.glowIntensity === next) {
      return;
    }
    this.glowIntensity = next;
    this.dirty = true;
  }

  /**
   * 调整柔光半径（0.2..3，1 为默认）：按倍率放大高斯模糊的采样步长，
   * 半径越大越柔和；倍率过高时 5 抽样会出现轻微分层，属于性能取舍。
   */
  setGlowRadius(value: number): void {
    const next = Math.max(0.2, Math.min(3, value));
    if (this.glowRadius === next) {
      return;
    }
    this.glowRadius = next;
    this.dirty = true;
  }

  /**
   * 显示 / 隐藏单个节点。隐藏节点会跳过逐帧计算，其选中线框也会同步隐藏；
   * 状态变化后立刻重放当前帧，使可见性即时生效。
   */
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

  /**
   * 跳转到指定帧（会四舍五入并 clamp 到 [0, frameCount - 1]）。
   * 帧未变化时直接返回，避免拖动时间轴时重复写顶点与矩阵。
   */
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

  /**
   * “层深”倍率：不重新转换，只按当前帧重放矩阵并追加 Z 平移增量。
   * @param value 1 表示原始烘焙深度
   */
  setDepthScale(value: number): void {
    this.depthAmount = value;
    if (this.model != null) {
      this.applyFrame(Math.max(0, this.frameIndex));
      this.dirty = true;
    }
  }

  /**
   * “厚度”倍率：同样不重新转换，只改写矩阵的 Z 轴缩放分量。
   * @param value 1 表示原始挤出厚度
   */
  setThicknessScale(value: number): void {
    this.thicknessAmount = value;
    if (this.model != null) {
      this.applyFrame(Math.max(0, this.frameIndex));
      this.dirty = true;
    }
  }

  /** 开关自动环绕；视角锁定时忽略（两者互斥）。 */
  setAutoRotate(enabled: boolean): void {
    this.controls.autoRotate = enabled && !this.rotationLocked;
    this.dirty = true;
  }

  /** 锁定环绕，但保留平移 / 缩放与轴向切换，便于固定角度看细节。 */
  setRotationLock(locked: boolean): void {
    this.rotationLocked = locked;
    this.controls.enableRotate = !locked;
    if (locked) {
      this.controls.autoRotate = false;
    }
    this.updateGizmoSelection();
    this.dirty = true;
  }

  /**
   * 平滑地把相机吸附到六个正交视角之一，并保持当前观察距离。
   * @param axis 目标轴向（+X/-X/+Y/-Y/+Z/-Z）
   */
  viewAxis(axis: ViewAxis): void {
    const distance = Math.max(2.4, this.camera.position.distanceTo(this.controls.target));
    const to = this.controls.target.clone().add(AXIS_VECTORS[axis].clone().multiplyScalar(distance));
    // OrbitControls 的球坐标转换在 Y 轴极点会退化，因此顶视图 / 底视图
    // 给 Z 方向加一点点偏移（约 0.23°），避免 up 向量与视线共线。
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
   * 根据 FPS 读数自适应降低渲染分辨率：连续 6 个低帧窗口（< 45fps）
   * 才降一次 pixelRatio（×0.8，下限 0.75）；流畅时清零计数。
   * 由 App 的 FPS 统计每 0.5s 调用一次，因此在几秒内收敛。
   *
   * @param fps 最近一个统计窗口的平均帧率
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

  /** 回到默认 +Z 视角（同时取消轴向高亮与未完成的补间）。 */
  resetView(): void {
    this.selectedAxis = null;
    this.viewTween = null;
    this.camera.position.copy(AXIS_HOME);
    this.controls.target.set(0, 0, 0);
    this.controls.update();
    this.updateGizmoSelection();
    this.dirty = true;
  }

  /**
   * 每帧调用一次（由 App 的 rAF 时钟驱动）。
   * 有视角补间时先推进插值；随后只有“相机移动或 dirty”才真正绘制，
   * 暂停且视角静止时不产生任何 GPU 开销。
   */
  render(): void {
    const now = performance.now();
    // 轴向切换补间：三次缓出，320ms 内插值到目标位置。
    if (this.viewTween) {
      const t = Math.min(1, (now - this.viewTween.started) / VIEW_TWEEN_MS);
      const eased = 1 - (1 - t) ** 3;
      this.camera.position.lerpVectors(this.viewTween.from, this.viewTween.to, eased);
      if (t >= 1) {
        this.viewTween = null;
      }
      this.dirty = true;
    }

    // OrbitControls.update() 返回是否发生了移动（含阻尼惯性）。
    const moved = this.controls.update();
    if (!moved && !this.dirty) {
      return;
    }

    const width = Math.max(1, this.host.clientWidth);
    const height = Math.max(1, this.host.clientHeight);
    // 相机可能刚移动：透视补偿依赖相机位姿，必须重写各层世界矩阵。
    this.writeMatrices();
    this.updateSelectionBoxes();
    // 主场景：先铺满视口清屏，再绘制；柔光与 gizmo 叠加在其上。
    this.renderer.setViewport(0, 0, width, height);
    this.renderer.setScissorTest(false);
    this.renderer.clear();
    this.renderer.render(this.scene, this.camera);
    this.renderGlow(width, height);
    this.renderGizmo(width, height);
    this.dirty = false;
  }

  /** 释放所有 GPU / DOM 资源；调用后 Stage 不可再用。 */
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

  /**
   * 写入所有图层的世界矩阵：轨道取值 → 层深 / 厚度倍率 → 透视补偿。
   *
   * 透视补偿的由来：层深被烘焙进矩阵的 Z 平移，而相机是 40° 透视，
   * 于是越远的图层被压向画面中心、越近的被推离中心（离中心越远偏差越大），
   * 平移动画里各层相互错位，与 Lottie 原画对不上。这里把每层的原点换到
   * 相机空间，按沿视线方向的深度 d 做一次 XY 缩放：
   * `k = d / 相机到观察目标的距离`，投影因此退化为统一尺度的正交投影，
   * 任意视角下各层位置都与原画重合；层深仍保留在 Z 上负责遮挡与排序，
   * 绕转视角时也会照常显出前后错位（错位来自旋转而非透视除法）。
   */
  private writeMatrices(): void {
    if (this.model == null) {
      return;
    }
    this.camera.updateMatrixWorld();
    this.cameraInverse.copy(this.camera.matrixWorldInverse);
    // 参考平面：过观察目标、垂直于视线的平面（默认即原画所在的 Z=0 平面）。
    this.referenceDistance = Math.max(
      1e-4,
      this.camera.position.distanceTo(this.controls.target),
    );

    // 每条轨道是帧连续的 16 float 块，这里算好当前帧的起点。
    const trackOffset = Math.min(this.frameIndex, this.model.frameCount - 1) * 16;
    // 层深与厚度都是“倍率”，1 表示保持烘焙值。
    const depthDelta = this.depthAmount - 1;
    const thickness = this.thicknessAmount;
    const cameraMatrix = this.camera.matrixWorld;

    for (let id = 0; id < this.entries.length; id += 1) {
      const entry = this.entries[id];
      const matrix = entry.mesh.matrixWorld;
      // 所有图层都挂在场景根节点下，没有父级矩阵需要相乘；常量轨道复用
      // 预先算好的 base，避免每帧从 Float32Array 重新组装。
      if (entry.transformStatic) {
        matrix.copy(entry.base);
      } else if (entry.track != null) {
        matrix.fromArray(entry.track, trackOffset);
      }
      const elements = matrix.elements;
      // 深度已经烘焙进轨道的 Z 平移（列主序第 14 个元素）：
      // - 滑块按 data.depth * (倍率 - 1) 追加增量；
      // - 拖拽重排的 stackDelta 也随倍率缩放，二者共同得到目标层深。
      const depthShift = entry.data.depth * depthDelta + entry.stackDelta * this.depthAmount;
      if (depthShift !== 0) {
        elements[14] += depthShift;
      }
      // 同层共面网格的绘制层序：后画的网格沿 +Z（朝相机）微移，消除共面闪烁。
      if (entry.stackOffset !== 0) {
        elements[14] += entry.stackOffset;
      }
      if (thickness !== 1) {
        // 等价于在矩阵右侧乘 scale(1, 1, thickness)：只改第三列（Z 基向量）的分量。
        elements[2] *= thickness;
        elements[6] *= thickness;
        elements[10] *= thickness;
      }

      // 图层原点在相机空间的深度（相机朝 -Z 看，故取负 z）。
      this.cameraSpacePoint
        .set(elements[12], elements[13], elements[14])
        .applyMatrix4(this.cameraInverse);
      const depth = -this.cameraSpacePoint.z;
      if (depth <= 1e-4) {
        continue;
      }
      const scale = depth / this.referenceDistance;
      // 落在参考平面上的图层无需补偿（绝大多数静态层走这条捷径）。
      if (scale > 0.999 && scale < 1.001) {
        continue;
      }
      // matrix = cameraMatrix · diag(scale, scale, 1) · cameraInverse · matrix
      this.matrixScratch.multiplyMatrices(this.cameraInverse, matrix);
      const scaled = this.matrixScratch.elements;
      // 左乘 diag(scale, scale, 1, 1) 等价于把前两行按 scale 缩放
      // （列主序下每列 4 个元素，前两个即第 0、1 行）。
      for (let col = 0; col < 16; col += 4) {
        scaled[col] *= scale;
        scaled[col + 1] *= scale;
      }
      matrix.multiplyMatrices(cameraMatrix, this.matrixScratch);
    }
  }

  /**
   * 逐帧核心：写颜色、顶点与索引桶（世界矩阵由 [`writeMatrices`] 负责）。
   * 只做内存拷贝和少量标量运算，不做任何几何计算。
   *
   * @param frame 目标帧（调用方已保证在 [0, frameCount - 1] 内）
   */
  private applyFrame(frame: number): void {
    if (this.model == null) {
      return;
    }
    this.frameIndex = frame;
    // 换帧后立即刷新矩阵，保证调用方读到一致的世界矩阵；相机移动时
    // 由 render() 再调用一次以更新透视补偿。
    this.writeMatrices();

    for (let id = 0; id < this.entries.length; id += 1) {
      const entry = this.entries[id];
      const { data, mesh, material, position, colors } = entry;

      // 隐藏节点跳过全部逐帧工作（矩阵、颜色、顶点、索引），直到重新显示。
      if (this.hiddenNodes.has(id)) {
        mesh.visible = false;
        continue;
      }

      // 图片贴图尚未就绪（仍在加载或已失败）时保持隐藏。
      if (entry.textured && !entry.textureReady) {
        mesh.visible = false;
        continue;
      }

      // 逐帧颜色：每帧 4 个 float（RGBA），alpha 过低直接整帧不可见。
      if (colors != null) {
        const colorBase = frame * 4;
        const alpha = colors[colorBase + 3];
        if (alpha < 0.004) {
          mesh.visible = false;
          continue;
        }
        mesh.visible = true;
        material.opacity = alpha;
        // 颜色按 sRGB 输入，交给 three.js 转到线性空间后再参与光照。
        material.color.setRGB(
          colors[colorBase],
          colors[colorBase + 1],
          colors[colorBase + 2],
          THREE.SRGBColorSpace,
        );
      } else {
        mesh.visible = entry.staticVisible;
      }

      // 逐帧顶点：v2 用 frameOffsets 定位本帧区间；区间单位是 float，不是顶点。
      // 不同拓扑桶的帧顶点数可以不同，因此不要假设固定长度。
      if (entry.positionsAnimated && data.framePositions) {
        const offsets = data.frameOffsets;
        if (offsets != null) {
          const start = offsets[frame] ?? 0;
          const end = offsets[frame + 1] ?? start;
          if (end > start) {
            // subarray 是视图，set 会直接写进现有的 position 数组（零临时分配）。
            position.array.set(data.framePositions.subarray(start, end));
            position.needsUpdate = true;
          }
        } else {
          // 兼容旧布局：每帧预留“最大顶点数 × 3”个 float。
          const count = data.vertexCount * 3;
          const offset = frame * count;
          if (offset + count <= data.framePositions.length) {
            position.array.set(data.framePositions.subarray(offset, offset + count));
            position.needsUpdate = true;
          }
        }
      }

      // 拓扑切换：frameBuckets[frame] 指定本帧使用哪套索引。
      // 仅在桶变化时才 setIndex（会替换 geometry.index 的绑定），
      // 桶下标越界时回退到桶 0，保证不会引用到未创建的属性。
      const buckets = entry.indexAttributes;
      if (buckets.length > 1) {
        const bucket = data.frameBuckets?.[frame] ?? 0;
        const wanted = bucket < buckets.length ? bucket : 0;
        if (wanted !== entry.currentBucket) {
          entry.currentBucket = wanted;
          entry.geometry.setIndex(buckets[wanted]);
        }
      }
    }
  }

  /** 清空所有网格、线框与柔光外壳（切换模型时调用）。 */
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
      // 图片网格的纹理由 TextureLoader 创建，需要一并释放。
      entry.material.map?.dispose();
      // 渐变 LUT 由 Stage 自己创建，也需释放。
      entry.gradientLut?.dispose();
    }
    this.entries.length = 0;
    this.nodes.length = 0;
    this.hiddenNodes.clear();
    this.model = null;
  }

  /** 容器尺寸变化：同步画布分辨率（CSS 尺寸由样式控制，故 setSize 第三参为 false）。 */
  private resize(): void {
    const width = Math.max(1, this.host.clientWidth);
    const height = Math.max(1, this.host.clientHeight);
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.dirty = true;
  }

  // ------------------------------------------------------- 选择线框与柔光

  /**
   * 计算只覆盖“当前桶实际使用顶点”的紧包围盒。
   * 顶点数组中可能残留其它桶的顶点（缓冲取最大值分配），
   * 因此必须用 vertexCount 限制扫描范围。
   *
   * @param box 输出包围盒（复用对象）
   * @param array xyz 交错的顶点数组
   * @param vertexCount 本桶顶点数
   */
  private setBoxFromPositions(
    box: THREE.Box3,
    array: ArrayLike<number>,
    vertexCount: number,
  ): void {
    box.makeEmpty();
    const limit = Math.min(vertexCount, Math.floor(array.length / 3)) * 3;
    const { min, max } = box;
    for (let i = 0; i < limit; i += 3) {
      const x = array[i];
      const y = array[i + 1];
      const z = array[i + 2];
      if (x < min.x) min.x = x;
      if (y < min.y) min.y = y;
      if (z < min.z) min.z = z;
      if (x > max.x) max.x = x;
      if (y > max.y) max.y = y;
      if (z > max.z) max.z = z;
    }
  }

  /**
   * 每次绘制前刷新所有选中线框：顶点数据可能刚被换帧覆盖，
   * 因此不能只在选择变化时计算一次。
   */
  private updateSelectionBoxes(): void {
    for (const [id, helper] of this.selectionBoxes) {
      const entry = this.entries[id];
      if (entry == null || !entry.mesh.visible) {
        helper.visible = false;
        continue;
      }
      // 用“当前桶”的顶点数而非全局最大值，避免把上一桶残留的顶点算进包围盒。
      const bucket = entry.data.buckets[entry.currentBucket] ?? entry.data.buckets[0];
      this.setBoxFromPositions(
        this.boxScratch,
        entry.position.array,
        bucket ? bucket.vertexCount : entry.data.vertexCount,
      );
      // 顶点是局部坐标，需乘上已更新的世界矩阵才是场景空间包围盒。
      this.boxScratch.applyMatrix4(entry.mesh.matrixWorld);
      if (this.boxScratch.isEmpty()) {
        helper.visible = false;
        continue;
      }
      helper.box.copy(this.boxScratch);
      helper.visible = true;
    }
  }

  /**
   * 柔光的“总开关 + 单节点覆盖”语义：
   * - 总开关开启：除非节点被显式标记 false，否则都发光；
   * - 总开关关闭：只有显式标记 true 的节点发光（可单点亮）。
   */
  private isNodeGlowEnabled(id: number): boolean {
    return this.softGlow
      ? this.glowFlags.get(id) !== false
      : this.glowFlags.get(id) === true;
  }

  /**
   * 按当前发光集合增删外壳网格：只为启用的节点创建，
   * 关闭的节点立即移出 glowScene 并释放材质（geometry 与主网格共享，不能释放）。
   */
  private syncGlowMeshes(): void {
    const wanted = new Set<number>();
    for (let id = 0; id < this.entries.length; id += 1) {
      // 图片贴图网格没有可发光的纯色几何，跳过以免出现白色外壳。
      if (this.entries[id].textured) {
        continue;
      }
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
      // 外壳材质：叠加混合、关闭深度写入，颜色与透明度每帧从主材质同步。
      // 复用主网格的 geometry，因此顶点 / 索引桶切换会自动跟主画面保持一致。
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

  /** 移除全部柔光外壳（只释放材质，geometry 仍归主网格）。 */
  private clearGlowMeshes(): void {
    for (const mesh of this.glowMeshes.values()) {
      this.glowScene.remove(mesh);
      (mesh.material as THREE.Material).dispose();
    }
    this.glowMeshes.clear();
  }

  /**
   * 按主画布的 1/4 尺寸准备两张渲染目标；尺寸变化时重建。
   * 不使用深度/模板缓冲，降低显存占用。
   *
   * @returns 是否可用（当前实现总会返回 true，便于调用方提前退出）
   */
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
   * 加性柔光后处理：只渲染发光外壳 → 1/4 分辨率做两遍高斯模糊 → 叠加回主画面。
   *
   * 流程：外壳同步矩阵/颜色 → 渲染到 read → 横向模糊到 write
   *       → 纵向模糊回 read → 以加性混合把 read.texture 铺满视口。
   * 只要有一个节点发光就会执行，即使总柔光开关关闭。
   */
  private renderGlow(width: number, height: number): void {
    if (this.glowMeshes.size === 0 || this.glowIntensity <= 0) {
      return;
    }
    this.ensureGlowTargets(width, height);
    const read = this.glowRead;
    const write = this.glowWrite;
    if (read == null || write == null) {
      return;
    }

    // 外壳与主网格共享 geometry，逐帧只需同步世界矩阵、可见性与颜色。
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

    // 不透明黑底：先清空颜色缓冲，保证加性叠加时不会残留上一帧的亮度。
    this.renderer.setClearColor(0x000000, 1);
    this.renderer.setScissorTest(false);
    this.renderer.setRenderTarget(read);
    this.renderer.clear(true, false, false);
    this.renderer.render(this.glowScene, this.camera);

    // 第一遍模糊：横向，步长 = 半径倍率 / 目标宽度（纹理坐标单位）。
    this.glowQuad.material = this.glowBlurMaterial;
    this.glowBlurMaterial.uniforms.source.value = read.texture;
    (this.glowBlurMaterial.uniforms.direction.value as THREE.Vector2).set(
      this.glowRadius / this.glowWidth,
      0,
    );
    this.renderer.setRenderTarget(write);
    this.renderer.clear(true, false, false);
    this.renderer.render(this.glowQuadScene, this.glowQuadCamera);

    // 第二遍模糊：纵向，读第一遍的结果写回 read，避免额外一张纹理。
    this.glowBlurMaterial.uniforms.source.value = write.texture;
    (this.glowBlurMaterial.uniforms.direction.value as THREE.Vector2).set(
      0,
      this.glowRadius / this.glowHeight,
    );
    this.renderer.setRenderTarget(read);
    this.renderer.clear(true, false, false);
    this.renderer.render(this.glowQuadScene, this.glowQuadCamera);

    // 合成材质延迟创建并复用；只更新贴图，避免每帧重编译着色器。
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
    // 强度倍率只影响叠加不透明度，样式参数变化无需重建材质。
    composite.opacity = GLOW_OPACITY * this.glowIntensity;

    // 回到主画布：只清深度（保留已渲染的颜色），再用加性混合叠加柔光。
    this.renderer.setRenderTarget(null);
    this.renderer.setViewport(0, 0, width, height);
    this.renderer.setScissorTest(false);
    this.renderer.clearDepth();
    this.glowQuad.material = composite;
    this.renderer.render(this.glowQuadScene, this.glowQuadCamera);
    // 恢复透明清屏色，供下一帧主场景使用。
    this.renderer.setClearColor(0x000000, 0);
  }

  /** 释放柔光相关的 RT、合成材质、全屏四边形与着色器。 */
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

  // -------------------------------------------------------------- 轴向 gizmo

  /** gizmo 的边长（CSS px）：取容器短边的 28%，限制在 76–128。 */
  private get gizmoSize(): number {
    const shortest = Math.min(this.host.clientWidth, this.host.clientHeight);
    return Math.max(76, Math.min(128, Math.floor(shortest * 0.28)));
  }

  /**
   * 构建 gizmo：半透明底盘 + 三条轴线 + 六个球 + 方向标签 + 选中环。
   * 所有资源都压入 gizmoResources / gizmoTextures，dispose 时统一释放。
   */
  private buildGizmo(): void {
    // 底盘：径向渐变圆形贴图，弱化背景以突出轴向球。
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

    // 六个轴向球共享同一份球体几何，只按轴替换材质。
    const ballGeometry = new THREE.SphereGeometry(0.19, 20, 14);
    this.gizmoResources.push(ballGeometry);

    // 三条轴线：从负轴到正轴各画一条淡色线段。
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

    // 每个轴向：一个彩色球 + 一个文字标签（Sprite 始终面向相机）。
    // 负轴用半透明材质，正负方向一眼可辨。
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

    // 选中环：默认隐藏，点击 / 切换轴向时套在当前轴上。
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

  /**
   * 刷新 gizmo 的选中反馈：当前轴的球放大到 1.3 倍并套上圆环；
   * 圆环颜色区分“自由视角”（青）与“视角锁定”（黄）。
   */
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

  /**
   * 用 scissor 把 gizmo 叠加到主画面左上角（注意 WebGL 视口原点在左下角，
   * 因此 y = height - size - margin）。
   */
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

  /**
   * 让 gizmo 相机跟随主相机：位置沿主视线方向后退 6 个单位，
   * 朝向与主相机一致，这样轴向球的投影方向与真实视角同步。
   */
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

  /** 把指针的页面坐标换算成容器内坐标（左上角为原点，单位 CSS px）。 */
  private localPoint(event: PointerEvent): { x: number; y: number } {
    const rect = this.host.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  /** 判断容器坐标是否落在左上角 gizmo 的方形区域内。 */
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

  /**
   * gizmo 轴向拾取：把六个轴向球投影到屏幕坐标，取距离点击点最近且在
   * 阈值（≥16px 或 20% 尺寸）内的轴；多轴重叠时优先更靠近观察者的一个。
   *
   * @param point 容器内点击坐标（CSS px）
   * @returns 命中的轴向；未命中返回 null
   */
  private pickGizmoAxis(point: { x: number; y: number }): ViewAxis | null {
    const size = this.gizmoSize;
    // gizmo 视口中心在容器坐标中的位置。
    const centerX = this.gizmoMargin + size * 0.5;
    const centerY = this.gizmoMargin + size * 0.5;
    const threshold = Math.max(16, size * 0.2);
    this.syncGizmoCamera();

    let best: ViewAxis | null = null;
    let bestDistance = Infinity;
    let bestDepth = -Infinity;

    for (const axis of AXIS_ORDER) {
      // 手动做一次 MVP 变换：视空间 z 用于判深度，NDC 坐标用于算像素距离。
      this.gizmoProjection.copy(AXIS_VECTORS[axis]);
      this.gizmoProjection.applyMatrix4(this.gizmoCamera.matrixWorldInverse);
      const depth = this.gizmoProjection.z;
      this.gizmoProjection.applyMatrix4(this.gizmoCamera.projectionMatrix);
      // NDC → 像素：y 轴需翻转（屏幕向下为正）。
      const px = centerX + this.gizmoProjection.x * size * 0.5;
      const py = centerY - this.gizmoProjection.y * size * 0.5;
      const distance = Math.hypot(px - point.x, py - point.y);
      if (distance > threshold) {
        continue;
      }
      // 轴向重叠时优先更靠近观察者的轴（视空间 z 更大，即更靠近相机）。
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

  /** 生成 gizmo 底盘贴图：径向渐变圆 + 一圈细描边（Canvas → CanvasTexture）。 */
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

  /** 生成轴向文字贴图：深色描边 + 白字，保证在任何背景下可读。 */
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

  /** 生成选中圆环贴图（白色细环，材质颜色再决定青 / 黄）。 */
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

  /** 释放 gizmo 的几何/材质/贴图与场景引用。 */
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
