/**
 * Lottie JSON 的最小类型定义。
 *
 * 这里只声明本项目用到的字段，其余字段通过索引签名保留，
 * 避免在拆分图层时丢失 Bodymovin 导出的额外信息。
 */

/** Lottie 属性：静态值或关键帧动画 */
export interface LottieProperty {
  /** 0 = 静态值（k 直接是值）；1 = 动画（k 是关键帧数组）。 */
  a: 0 | 1;
  /** 静态值或关键帧数组；具体结构由 Rust 侧 prop.rs 按 a 解析。 */
  k: unknown;
  /** 属性索引，对应 lottie-web 内部表达式的引用编号，本转换器只透传。 */
  ix?: number;
}

/** Lottie 图层（shape / null / solid / image / precomp 等） */
export interface LottieLayer {
  /** 图层 id；`parent` / `tt` 引用的是它而不是数组下标。 */
  ind: number;
  /** 图层类型：3 = null、4 = shape、0 = precomp、1 = solid、2 = image。 */
  ty: number;
  /** 图层名，会作为节点列表的显示名。 */
  nm: string;
  /** 变换属性集（a 锚点 / p 位置 / s 缩放 / r 旋转 / o 透明度 / sk 倾斜等）。 */
  ks: Record<string, LottieProperty>;
  /** 入点（源帧单位），本地时间 = 根时间 - st。 */
  ip: number;
  /** 出点（源帧单位，不含右端点）。 */
  op: number;
  /** 起始时间偏移，precomp 子层会再叠加父层时间伸缩。 */
  st: number;
  /** 3D 图层标记（1 表示启用 rz/rx/ry）。 */
  ddd?: number;
  /** 时间伸缩比，仅预合成层使用；0 按 1 处理。 */
  sr?: number;
  /** 自动定向标记。 */
  ao?: number;
  /** 混合模式。 */
  bm?: number;
  /** 父图层 ind；带父级时世界矩阵需要沿链左乘。 */
  parent?: number;
  /** 轨道遮罩类型：非 0/非 undefined 时遮住紧邻其上的图层。 */
  tt?: number;
  /** 隐藏标记，隐藏层不参与渲染。 */
  hd?: boolean;
  /** 引用的 assets id（预合成 / 图片）。 */
  refId?: string;
  /** 纯色层颜色（十六进制字符串）。 */
  sc?: string;
  /** 纯色层宽度。 */
  sw?: number;
  /** 纯色层高度。 */
  sh?: number;
  [key: string]: unknown;
}

/** 一个 Lottie 动画文件 */
export interface LottieData {
  /** 格式版本（Bodymovin 导出版本号，仅作信息）。 */
  v: string;
  /** 源帧率（fps），用于把 ip/op 换算成秒。 */
  fr: number;
  /** 动画入点（源帧）。 */
  ip: number;
  /** 动画出点（源帧）。 */
  op: number;
  /** 画布宽（px）。 */
  w: number;
  /** 画布高（px）。 */
  h: number;
  /** 动画名称。 */
  nm?: string;
  /** 是否 3D 合成。 */
  ddd?: number;
  /** 资源表（预合成、图片等）。 */
  assets?: unknown[];
  /** 顶层图层数组，顺序为「前 → 后」，索引 0 最靠前。 */
  layers: LottieLayer[];
  /** 时间轴标记（本转换器不消费）。 */
  markers?: unknown[];
  [key: string]: unknown;
}
