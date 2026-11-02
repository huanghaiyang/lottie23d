/**
 * Lottie JSON 的最小类型定义。
 *
 * 这里只声明本项目用到的字段，其余字段通过索引签名保留，
 * 避免在拆分图层时丢失 Bodymovin 导出的额外信息。
 */

/** Lottie 属性：静态值或关键帧动画 */
export interface LottieProperty {
  a: 0 | 1;
  k: unknown;
  ix?: number;
}

/** Lottie 图层（shape / null / solid / image / precomp 等） */
export interface LottieLayer {
  ind: number;
  ty: number;
  nm: string;
  ks: Record<string, LottieProperty>;
  ip: number;
  op: number;
  st: number;
  ddd?: number;
  sr?: number;
  ao?: number;
  bm?: number;
  parent?: number;
  tt?: number;
  hd?: boolean;
  refId?: string;
  sc?: string;
  sw?: number;
  sh?: number;
  [key: string]: unknown;
}

/** 一个 Lottie 动画文件 */
export interface LottieData {
  v: string;
  fr: number;
  ip: number;
  op: number;
  w: number;
  h: number;
  nm?: string;
  ddd?: number;
  assets?: unknown[];
  layers: LottieLayer[];
  markers?: unknown[];
  [key: string]: unknown;
}
