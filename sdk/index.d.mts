/**
 * lottie2mesh SDK 的 TypeScript 类型声明，与 `index.mjs` 的实际返回结构一一对应。
 *
 * 单位约定：
 * - `positions` / 变换矩阵：three.js 世界坐标（全局矩阵已按 `3.6 / max(w, h)` 归一化）；
 * - `depth`：归一化深度（约 -0.65..0.65），已烘焙进矩阵的 Z 平移；
 * - `thickness` / `halfThickness` / `edge_radius`：Lottie 画布像素；
 * - 颜色：sRGB 分量 0..1；`duration`：秒。
 */

/** 传给 Rust 转换器的选项；字段均可省略，缺省值由 Rust 侧决定。 */
export interface ConvertOptions {
  /** 烘焙采样帧率（6–120）；默认沿用文档帧率 `fr`。 */
  fps?: number;
  /** 烘焙帧数硬上限（默认 600，最大 1200）。 */
  max_frames?: number;
  /** 挤出半厚度，单位 Lottie 像素。 */
  thickness?: number;
  /** 边缘平滑半径，单位 Lottie 像素；仅填充生效，并 clamp 到半厚度。 */
  edge_radius?: number;
  /** `edge_radius` 的 camelCase 别名。 */
  edgeRadius?: number;
  /**
   * 轮廓采样间距，单位 Lottie 像素/点（1..8，默认 3）。
   * 数值越小采样点越多、曲线越平滑；受 `MAX_K = 8192` 与 8M float 预算约束。
   */
  curve_spacing?: number;
  /** `curve_spacing` 的 camelCase 别名。 */
  curveSpacing?: number;
}

/** 一套拓扑桶：同一采样轮廓布局下缓存的索引，播放时切桶只换 index。 */
export interface MeshBucket {
  /** 该拓扑的三角形索引（每 3 个 u32 一个三角形）。 */
  indices: Uint32Array;
  /** 索引数量（= indices.length）。 */
  indexCount: number;
  /** 该桶实际使用的顶点数；不同桶可以不同。 */
  vertexCount: number;
  /** 参考轮廓点数（未乘挤出剖面 stride）。 */
  pointCount: number;
}

/** 一个渐变颜色 stop：偏移 0..1 与 sRGB 颜色。 */
export interface GradientColorStop {
  /** 沿渐变轴的归一化位置（0..1）。 */
  offset: number;
  /** sRGB 颜色分量，各 0..1。 */
  color: [number, number, number];
}

/** 一个渐变透明度 stop：偏移 0..1 与 alpha。 */
export interface GradientAlphaStop {
  /** 沿渐变轴的归一化位置（0..1）。 */
  offset: number;
  /** 透明度 0..1。 */
  alpha: number;
}

/**
 * 渐变求值规格（仅渐变网格存在）。颜色由播放端在片元着色器里按局部坐标
 * 逐像素求值，避免逐顶点采样过疏导致的硬边与三角形接缝。
 */
export interface GradientModel {
  /** 渐变类型：1 = 线性，2 = 径向。 */
  kind: number;
  /** 渐变起点（线性 0% / 径向圆心），已随组变换烘焙到网格局部坐标。 */
  start: [number, number];
  /** 渐变终点（线性 100% / 径向半径端点）。 */
  end: [number, number];
  /** 颜色 stop，按偏移升序。 */
  stops: GradientColorStop[];
  /** 透明度 stop，按偏移升序；为空表示不透明。 */
  alphaStops: GradientAlphaStop[];
}

/** 单个可绘制网格（一条填充或描边），几何与逐帧数据都由 Rust 预烘焙。 */
export interface MeshModel {
  /** 在 `meshes` 数组中的下标；排序后同时也是节点 id。 */
  index: number;
  /** 网格类型：0 = 填充（fill），1 = 描边（stroke），2 = 图片贴图。 */
  kind: number;
  /** 是否带逐帧顶点轨道（存在 `framePositions`）。 */
  animated: boolean;
  /** 参考帧颜色 RGBA（sRGB，0..1），常量颜色时即整段使用的颜色。 */
  color: Float32Array;
  /** 归一化层深（约 -0.65..0.65），已烘焙进变换矩阵的 Z 平移。 */
  depth: number;
  /** 挤出半厚度，单位 Lottie 像素。 */
  halfThickness: number;
  /** 所有拓扑桶中的最大顶点数（顶点缓冲按它分配）。 */
  vertexCount: number;
  /** 桶 0 的索引数量；多桶时各桶可能不同。 */
  indexCount: number;
  /** 参考轮廓点数（未乘挤出剖面 stride）。 */
  pointCount: number;
  /** 参考帧顶点，xyz 交错（长度 = vertexCount × 3）。 */
  positions: Float32Array;
  /** 逐帧顶点，按帧连续打包；区间由 `frameOffsets` 指定，无动画时为 null。 */
  framePositions: Float32Array | null;
  /** 每帧在 `framePositions` 中的起点，单位是 **float 下标**，共 frameCount + 1 项。 */
  frameOffsets: Uint32Array | null;
  /** 每帧使用的拓扑桶下标（u8），仅多桶时存在。 */
  frameBuckets: Uint8Array | null;
  /** 各拓扑的缓存索引；始终至少 1 项，`buckets[0]` 与 `indices` 相同。 */
  buckets: MeshBucket[];
  /** 桶 0 的索引，方便只关心单一拓扑的调用方。 */
  indices: Uint32Array;
  /** 逐帧颜色 RGBA（帧数 × 4，sRGB），颜色恒定时为 null。 */
  colors: Float32Array | null;
  /**
   * 渐变求值规格，仅渐变网格存在。有值时 `color` 的 RGB 恒为 1（只保留
   * alpha），最终颜色 = 片元求值得到的渐变色 × `color`（含 alpha）。
   */
  gradient: GradientModel | null;
  /** 逐顶点 UV（长度 = vertexCount × 2），仅图片网格存在。 */
  uvs: Float32Array | null;
  /** 图片纹理地址（`u + p` 或 data URI），仅图片网格存在。 */
  texture: string | null;
  /** 引用的变换轨道下标（对应 `Model.transforms`）。 */
  transformIndex: number;
  /** three.js 绘制顺序（= ((depth + 1) * 10000) + 层内下标）。 */
  renderOrder: number;
  /** 图层名 + 可绘制对象名，例如 "Dot A · 光晕"。 */
  name: string;
}

/** 给 UI 使用的扁平节点信息；不携带顶点数据，避免列表持有大缓冲。 */
export interface NodeModel {
  /** 渲染顺序下标，同时也是 `meshes` 的下标。 */
  id: number;
  /** 图层名 + 可绘制对象名，例如 "Dot A · 光晕"。 */
  name: string;
  /** 网格类型：0 = 填充，1 = 描边。 */
  kind: number;
  /** 是否带逐帧顶点动画。 */
  animated: boolean;
  /** 跨桶最大顶点数。 */
  vertexCount: number;
  /** 桶 0 的索引数量。 */
  indexCount: number;
  /** three.js 绘制顺序。 */
  renderOrder: number;
}

/** 解析后的完整模型：header 元信息 + 变换轨道 + 网格 + 节点列表。 */
export interface Model {
  /** `.l2d3` 格式版本，当前为 4（新增图片贴图）。 */
  version: number;
  /** Lottie 画布宽（px）。 */
  width: number;
  /** Lottie 画布高（px）。 */
  height: number;
  /** 文档帧率（`fr`）。 */
  frameRate: number;
  /** 源动画首帧。 */
  ip: number;
  /** 源动画尾帧（不含）。 */
  op: number;
  /** 实际烘焙采样帧率；预算降采样时会低于请求值。 */
  sampleFps: number;
  /** 烘焙帧数（至少 1）。 */
  frameCount: number;
  /** 网格数量。 */
  meshCount: number;
  /** 变换轨道数量。 */
  transformCount: number;
  /** 所有网格的静态顶点总数。 */
  totalVertices: number;
  /** 所有拓扑桶的索引总数。 */
  totalIndices: number;
  /** `.l2d3` 文件总字节数。 */
  totalSize: number;
  /** 原生构建的转换耗时（微秒）；wasm 构建为 0。 */
  conversionUs: number;
  /** 文档名（Lottie 的 `nm`）。 */
  name: string;
  /** 动画时长（秒）= frameCount / sampleFps。 */
  duration: number;
  /** 每条变换轨道是 frameCount × 16 个 float，列主序 4×4 世界矩阵。 */
  transforms: Float32Array[];
  /** 已按 renderOrder 升序排序的网格列表。 */
  meshes: MeshModel[];
  /** 按渲染顺序排列的扁平节点列表；`id` 即 `meshes` 下标。 */
  nodes: NodeModel[];
}

/** `loadConverter()` 返回的转换器实例。 */
export interface Converter {
  /** 转换并解析为模型对象。 */
  convert(lottie: string | object, options?: ConvertOptions): Model;
  /** 只返回 `.l2d3` 字节，不解析（适合落盘 / 缓存）。 */
  convertBytes(lottie: string | object, options?: ConvertOptions): Uint8Array;
  /** 把已有 `.l2d3` 字节解析为模型，不依赖 wasm。 */
  parseModel(bytes: Uint8Array | ArrayBuffer): Model;
  /** wasm 的 C ABI 版本，与二进制 version 一致。 */
  version: number;
}

/**
 * 加载并实例化 wasm 转换器；同一实例的多次 convert 复用同一块线性内存。
 * @param source 可选；默认 Node 读 `public/wasm`，浏览器读 `document.baseURI + wasm/`。
 */
export function loadConverter(
  source?: string | URL | ArrayBuffer | ArrayBufferView | WebAssembly.Module,
): Promise<Converter>;

/** 独立导出的解析函数，效果等同于 `converter.parseModel`。 */
export function parseModel(bytes: Uint8Array | ArrayBuffer): Model;

declare const _default: typeof loadConverter;
export default _default;
