import type { LottieData, LottieLayer, LottieProperty } from './lottie/types';

/**
 * 内置示例动画。
 *
 * 这两个 Lottie 文件由代码生成，保证离线可用；
 * 每个示例都刻意使用多个独立图层，方便观察「分层 → 3D」的效果。
 */

/** 源帧率（fps）。 */
const FRAME_RATE = 60;
/** 总帧数：180 帧 / 60fps = 3 秒循环。 */
const TOTAL_FRAMES = 180; // 3 秒循环

type Numeric = number | number[];

/** 三次贝塞尔缓动控制点（x 是时间轴、y 是值轴），逐维度可不同。 */
interface Ease {
  x: number[];
  y: number[];
}

/** 单个关键帧：t 为源帧号，s 为该帧取值，i/o 为进/出缓动。 */
interface Keyframe {
  t: number;
  s: number[];
  i: Ease;
  o: Ease;
}

/** `a = 1` 的动画属性（k 为关键帧数组），与 Lottie 的动画属性结构一致。 */
interface AnimatedProperty {
  a: 1;
  k: Keyframe[];
  ix?: number;
}

/** 图层变换的可选入参：既可直接给静态值，也可给关键帧动画。 */
type TransformOptions = {
  pos?: number[] | AnimatedProperty;
  rot?: number | AnimatedProperty;
  scale?: number[] | AnimatedProperty;
  opacity?: number | AnimatedProperty;
  anchor?: number[];
};

/** 线性缓动：控制点固定在 (0.5, 0.5)，等价于匀速插值。 */
const linear = (dims: number): Ease => ({
  x: new Array(dims).fill(0.5),
  y: new Array(dims).fill(0.5),
});

/** 构造静态属性（a = 0），ix 固定为 1 仅为结构完整。 */
function staticProp(value: Numeric): LottieProperty {
  return { a: 0, k: value, ix: 1 };
}

/** 由若干关键帧构造线性动画属性 */
function animate(frames: Array<{ t: number; v: number[] }>): AnimatedProperty {
  const dims = frames[0].v.length;
  const i = linear(dims);
  const o = linear(dims);
  return {
    a: 1,
    k: frames.map((frame) => ({ t: frame.t, s: frame.v, i, o })),
    ix: 1,
  };
}

/** 类型守卫：判断传入的是否为关键帧动画属性（a === 1）。 */
function isAnimated(value: unknown): value is AnimatedProperty {
  return typeof value === 'object' && value !== null && (value as AnimatedProperty).a === 1;
}

/**
 * 组装 Lottie 的 `ks` 变换对象。
 * 每项都先判断是否动画属性，静态值再走 staticProp；缺省值按 Lottie 习惯补 100/0。
 */
function transform(options: TransformOptions): Record<string, LottieProperty> {
  return {
    o: isAnimated(options.opacity) ? options.opacity : staticProp(options.opacity ?? 100),
    r: isAnimated(options.rot) ? options.rot : staticProp(options.rot ?? 0),
    p: isAnimated(options.pos) ? options.pos : staticProp(options.pos ?? [0, 0, 0]),
    a: staticProp(options.anchor ?? [0, 0, 0]),
    s: isAnimated(options.scale) ? options.scale : staticProp(options.scale ?? [100, 100, 100]),
  };
}

/** 十六进制颜色转 0–1 的三元组（Lottie 的颜色分量按归一化浮点存储）。 */
function hexToRgb(hex: string): [number, number, number] {
  const normalized = hex.replace('#', '');
  const value = parseInt(normalized, 16);
  return [
    ((value >> 16) & 255) / 255,
    ((value >> 8) & 255) / 255,
    (value & 255) / 255,
  ];
}

/** 椭圆造型的可选参数。 */
interface EllipseOptions {
  /** 描边宽度（像素）；大于 0 时画描边，否则画填充。 */
  stroke?: number;
  /** 图层不透明度（0–100）。 */
  opacity?: number;
  /** 椭圆自身旋转角度（度）。 */
  rotation?: number;
  /** 填充色的 alpha（0–1），用于做出半透明光晕。 */
  alpha?: number;
}

/** Lottie 的矢量组（`gr`），it 内依次是形状、样式与组变换。 */
interface VectorGroup {
  ty: 'gr';
  it: unknown[];
  nm: string;
  np: number;
  cix: number;
  bm: number;
  ix: number;
  mn: string;
  hd: boolean;
}

/** 构造椭圆路径项（`el`），size 为宽高，position 为相对组内原点。 */
function ellipsePath(size: [number, number], position: [number, number] = [0, 0]): unknown {
  return {
    ty: 'el',
    d: 1,
    s: staticProp([...size]),
    p: staticProp([...position]),
    nm: 'Ellipse Path 1',
    mn: 'ADBE Vector Shape - Ellipse',
    hd: false,
  };
}

/** 构造组变换项（`tr`）：平移/锚点/缩放/旋转/透明度/倾斜全部保持默认。 */
function groupTransform(rotation = 0): unknown {
  return {
    ty: 'tr',
    p: staticProp([0, 0]),
    a: staticProp([0, 0]),
    s: staticProp([100, 100]),
    r: staticProp(rotation),
    o: staticProp(100),
    sk: staticProp(0),
    sa: staticProp(0),
    nm: 'Transform',
  };
}

/** 把形状与样式项包成一个矢量组，np 记录子项数量。 */
function makeGroup(items: unknown[]): VectorGroup {
  return {
    ty: 'gr',
    it: items,
    nm: 'Vector Group',
    np: items.length,
    cix: 2,
    bm: 0,
    ix: 1,
    mn: 'ADBE Vector Group',
    hd: false,
  };
}

/** 圆形 / 圆环（stroke > 0 时绘制描边，否则填充） */
function ellipseGroup(
  size: [number, number],
  color: string,
  options: EllipseOptions = {},
): VectorGroup {
  const { stroke = 0, opacity = 100, rotation = 0, alpha = 1 } = options;
  const rgb = hexToRgb(color);
  const items: unknown[] = [ellipsePath(size)];

  if (stroke > 0) {
    items.push({
      ty: 'st',
      c: staticProp([...rgb, 1]),
      o: staticProp(opacity),
      w: staticProp(stroke),
      lc: 2,
      lj: 2,
      ml: 4,
      bm: 0,
      nm: 'Stroke 1',
      mn: 'ADBE Vector Graphic - Stroke',
      hd: false,
    });
  } else {
    items.push({
      ty: 'fl',
      c: staticProp([...rgb, alpha]),
      o: staticProp(opacity),
      r: 1,
      bm: 0,
      nm: 'Fill 1',
      mn: 'ADBE Vector Graphic - Fill',
      hd: false,
    });
  }

  items.push(groupTransform(rotation));
  return makeGroup(items);
}

/** 带 trim path 的圆弧，可用于制作旋转的光弧 */
function arcGroup(
  size: [number, number],
  color: string,
  width: number,
  trimPercent: number,
  offset: AnimatedProperty,
  opacity = 100,
): VectorGroup {
  const rgb = hexToRgb(color);
  return makeGroup([
    ellipsePath(size),
    {
      ty: 'tm',
      s: staticProp(0),
      e: staticProp(trimPercent),
      o: offset,
      m: 1,
      nm: 'Trim Paths 1',
      mn: 'ADBE Vector Filter - Trim',
      hd: false,
    },
    {
      ty: 'st',
      c: staticProp([...rgb, 1]),
      o: staticProp(opacity),
      w: staticProp(width),
      lc: 2,
      lj: 2,
      ml: 4,
      bm: 0,
      nm: 'Stroke 1',
      mn: 'ADBE Vector Graphic - Stroke',
      hd: false,
    },
    groupTransform(),
  ]);
}

/** 图层附加项：父级 ind 与（保留的）透明度动画入参。 */
interface LayerOptions {
  parent?: number;
  opacity?: AnimatedProperty;
}

/**
 * 构造形状图层（ty = 4）。
 * @param ind 图层 id（parent 引用的就是它）
 * @param shapes 该层的形状组数组
 * @param options 变换参数
 * @param extra 父级等附加设置
 */
function shapeLayer(
  ind: number,
  name: string,
  shapes: unknown[],
  options: TransformOptions,
  extra: LayerOptions = {},
): LottieLayer {
  return {
    ddd: 0,
    ind,
    ty: 4,
    nm: name,
    sr: 1,
    ks: transform(options),
    ao: 0,
    shapes,
    ip: 0,
    op: TOTAL_FRAMES,
    st: 0,
    bm: 0,
    ...(extra.parent !== undefined ? { parent: extra.parent } : {}),
  };
}

/** 构造空对象层（ty = 3），只提供变换，常用于让子层绕其旋转。 */
function nullLayer(ind: number, name: string, options: TransformOptions): LottieLayer {
  return {
    ddd: 0,
    ind,
    ty: 3,
    nm: name,
    sr: 1,
    ks: transform(options),
    ao: 0,
    ip: 0,
    op: TOTAL_FRAMES,
    st: 0,
    bm: 0,
  };
}

/** 确定性伪随机，保证每次生成的星空位置一致 */
function seeded(index: number, salt: number): number {
  const value = Math.sin(index * 127.1 + salt * 311.7) * 43758.5453;
  return value - Math.floor(value);
}

/** 构造闪烁动画：由 base 出发，在 45/135 帧处增亮、90 帧处回落，180 帧回到起点。 */
function twinkle(base: number, index: number): AnimatedProperty {
  const peak = Math.min(100, base + 35 + seeded(index, 3) * 30);
  const mid = Math.max(12, base - 14);
  return animate([
    { t: 0, v: [base] },
    { t: 45, v: [peak] },
    { t: 90, v: [mid] },
    { t: 135, v: [peak * 0.8] },
    { t: 180, v: [base] },
  ]);
}

/* ------------------------------------------------------------------ 示例一：星轨 */

/**
 * 生成“星轨”示例：两组行星绕各自空对象旋转，外加轨道参考线、恒星与星空。
 * 图层顺序刻意前（行星、恒星）后（星云）分明，便于观察分层挤出效果。
 */
function createOrbitSample(): LottieData {
  const layerList: LottieLayer[] = [];
  let ind = 1;

  // 金色行星（挂在旋转的空对象上）
  layerList.push(
    shapeLayer(
      ind,
      'Aster',
      [ellipseGroup([92, 92], '#ffd7a0'), ellipseGroup([64, 64], '#ffe8be')],
      { pos: [196, 0, 0] },
      { parent: 100 },
    ),
  );
  ind += 1;

  layerList.push(
    shapeLayer(ind, 'Aster Ring', [ellipseGroup([176, 60], '#ffab5e', { stroke: 5, rotation: 18 })], {
      rot: animate([
        { t: 0, v: [12] },
        { t: 180, v: [372] },
      ]),
    }, { parent: 1 }),
  );
  ind += 1;

  layerList.push(
    shapeLayer(ind, 'Aster Moon', [ellipseGroup([22, 22], '#fff0d0')], {
      pos: [52, -42, 0],
    }, { parent: 1 }),
  );
  ind += 1;

  layerList.push(
    nullLayer(100, 'Orbiter A', {
      pos: [320, 320, 0],
      rot: animate([
        { t: 0, v: [0] },
        { t: 180, v: [360] },
      ]),
    }),
  );

  // 青色行星
  layerList.push(
    shapeLayer(ind, 'Cyan', [ellipseGroup([58, 58], '#8fe6ff')], { pos: [-150, 0, 0] }, { parent: 200 }),
  );
  ind += 1;

  layerList.push(
    shapeLayer(ind, 'Cyan Ring', [ellipseGroup([112, 34], '#63cfff', { stroke: 3, rotation: -24 })], {}, {
      parent: 4,
    }),
  );
  ind += 1;

  layerList.push(
    nullLayer(200, 'Orbiter B', {
      pos: [320, 320, 0],
      rot: animate([
        { t: 0, v: [0] },
        { t: 180, v: [-360] },
      ]),
    }),
  );

  // 轨道参考线
  layerList.push(
    shapeLayer(ind++, 'Orbit Guide 1', [ellipseGroup([400, 400], '#6d7fe8', { stroke: 1.4, opacity: 26 })], {
      pos: [320, 320, 0],
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Orbit Guide 2', [ellipseGroup([300, 300], '#6d7fe8', { stroke: 1.2, opacity: 20 })], {
      pos: [320, 320, 0],
    }),
  );

  // 恒星与光晕
  layerList.push(
    shapeLayer(ind++, 'Sun Glow Outer', [ellipseGroup([340, 340], '#ff8d5c', { alpha: 0.5 })], {
      pos: [320, 320, 0],
      opacity: 8,
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Sun Glow Inner', [ellipseGroup([212, 212], '#ffb877', { alpha: 0.7 })], {
      pos: [320, 320, 0],
      opacity: 14,
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Sun Core', [ellipseGroup([94, 94], '#fff0c2')], {
      pos: [320, 320, 0],
      scale: animate([
        { t: 0, v: [100, 100, 100] },
        { t: 90, v: [112, 112, 100] },
        { t: 180, v: [100, 100, 100] },
      ]),
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Sun Rim', [ellipseGroup([124, 124], '#ffe4a8', { stroke: 1.6, opacity: 46 })], {
      pos: [320, 320, 0],
    }),
  );

  // 星空
  for (let i = 0; i < 14; i++) {
    const x = 42 + seeded(i, 1) * 556;
    const y = 42 + seeded(i, 2) * 556;
    const size = 2 + seeded(i, 4) * 4.4;
    const color = i % 3 === 0 ? '#ffe8bf' : i % 3 === 1 ? '#dfe8ff' : '#9fd8ff';
    const base = 28 + seeded(i, 5) * 36;
    layerList.push(
      shapeLayer(
        ind++,
        `Star ${i + 1}`,
        [ellipseGroup([size, size], color)],
        { pos: [x, y, 0], opacity: twinkle(base, i) },
      ),
    );
  }

  // 星云（最后面的深度）
  layerList.push(
    shapeLayer(ind++, 'Nebula A', [ellipseGroup([1000, 820], '#5b63d8', { alpha: 0.55 })], {
      pos: [300, 292, 0],
      opacity: 12,
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Nebula B', [ellipseGroup([760, 760], '#7b4fd8', { alpha: 0.55 })], {
      pos: [430, 358, 0],
      opacity: 9,
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Nebula C', [ellipseGroup([520, 520], '#2f7fa8', { alpha: 0.7 })], {
      pos: [206, 424, 0],
      opacity: 11,
    }),
  );

  return {
    v: '5.13.0',
    fr: FRAME_RATE,
    ip: 0,
    op: TOTAL_FRAMES,
    w: 640,
    h: 640,
    nm: 'Cosmic Orbits',
    ddd: 0,
    assets: [],
    layers: layerList,
    markers: [],
  };
}

/* ------------------------------------------------------------------ 示例二：脉冲 */

/**
 * 生成“脉冲”示例：中心核缩放、三条 trim path 光弧反向旋转、外扩脉冲环，
 * 以及三组环绕光点（同样挂在旋转空对象上）。
 */
function createPulseSample(): LottieData {
  const layerList: LottieLayer[] = [];
  let ind = 1;

  // 中心核与光晕
  layerList.push(
    shapeLayer(ind++, 'Core Halo', [ellipseGroup([168, 168], '#7fe8ff', { alpha: 0.35 })], {
      pos: [360, 360, 0],
      opacity: 18,
      scale: animate([
        { t: 0, v: [92, 92, 100] },
        { t: 90, v: [126, 126, 100] },
        { t: 180, v: [92, 92, 100] },
      ]),
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Core', [ellipseGroup([62, 62], '#e8feff')], {
      pos: [360, 360, 0],
      scale: animate([
        { t: 0, v: [100, 100, 100] },
        { t: 90, v: [116, 116, 100] },
        { t: 180, v: [100, 100, 100] },
      ]),
    }),
  );

  // 三条不同半径的光弧
  const arcOffset = (direction: 1 | -1) =>
    animate([
      { t: 0, v: [0] },
      { t: 180, v: [360 * direction] },
    ]);

  layerList.push(
    shapeLayer(ind++, 'Arc A', [arcGroup([254, 254], '#7fe8ff', 3, 20, arcOffset(1))], {
      pos: [360, 360, 0],
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Arc B', [arcGroup([334, 334], '#8b96ff', 2.6, 14, arcOffset(-1))], {
      pos: [360, 360, 0],
      opacity: 86,
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Arc C', [arcGroup([424, 424], '#59e3e8', 2, 9, arcOffset(1))], {
      pos: [360, 360, 0],
      opacity: 62,
    }),
  );

  // 向外扩散的脉冲环
  layerList.push(
    shapeLayer(ind++, 'Pulse Ring A', [ellipseGroup([212, 212], '#7fe8ff', { stroke: 9 })], {
      pos: [360, 360, 0],
      scale: animate([
        { t: 0, v: [78, 78, 100] },
        { t: 15, v: [86, 86, 100] },
        { t: 90, v: [156, 156, 100] },
        { t: 180, v: [156, 156, 100] },
      ]),
      opacity: animate([
        { t: 0, v: [0] },
        { t: 15, v: [46] },
        { t: 90, v: [0] },
        { t: 180, v: [0] },
      ]),
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Pulse Ring B', [ellipseGroup([212, 212], '#8b96ff', { stroke: 7 })], {
      pos: [360, 360, 0],
      scale: animate([
        { t: 0, v: [78, 78, 100] },
        { t: 90, v: [78, 78, 100] },
        { t: 180, v: [156, 156, 100] },
      ]),
      opacity: animate([
        { t: 0, v: [0] },
        { t: 90, v: [0] },
        { t: 105, v: [40] },
        { t: 180, v: [0] },
      ]),
    }),
  );

  // 环绕的光点
  layerList.push(
    nullLayer(30, 'Ring 1', {
      pos: [360, 360, 0],
      rot: animate([
        { t: 0, v: [0] },
        { t: 180, v: [360] },
      ]),
    }),
  );
  layerList.push(
    shapeLayer(
      ind++,
      'Dot A',
      [ellipseGroup([30, 30], '#7fe8ff', { alpha: 0.2 }), ellipseGroup([10, 10], '#d9fbff')],
      { pos: [128, 0, 0] },
      { parent: 30 },
    ),
  );

  layerList.push(
    nullLayer(31, 'Ring 2', {
      pos: [360, 360, 0],
      rot: animate([
        { t: 0, v: [0] },
        { t: 180, v: [-360] },
      ]),
    }),
  );
  layerList.push(
    shapeLayer(
      ind++,
      'Dot B',
      [ellipseGroup([34, 34], '#8b96ff', { alpha: 0.2 }), ellipseGroup([11, 11], '#dfe4ff')],
      { pos: [212, 0, 0] },
      { parent: 31 },
    ),
  );

  layerList.push(
    nullLayer(32, 'Ring 3', {
      pos: [360, 360, 0],
      rot: animate([
        { t: 0, v: [0] },
        { t: 180, v: [360] },
      ]),
    }),
  );
  layerList.push(
    shapeLayer(
      ind++,
      'Dot C',
      [ellipseGroup([26, 26], '#59e3e8', { alpha: 0.18 }), ellipseGroup([8, 8], '#d6feff')],
      { pos: [296, 0, 0] },
      { parent: 32 },
    ),
  );

  // 深处闪烁的尘埃
  for (let i = 0; i < 12; i++) {
    const x = 50 + seeded(i, 11) * 620;
    const y = 50 + seeded(i, 12) * 620;
    const size = 2.2 + seeded(i, 13) * 3.4;
    layerList.push(
      shapeLayer(
        ind++,
        `Dust ${i + 1}`,
        [ellipseGroup([size, size], i % 2 === 0 ? '#7fe8ff' : '#aab6ff')],
        { pos: [x, y, 0], opacity: twinkle(24 + seeded(i, 14) * 30, i + 20) },
      ),
    );
  }

  layerList.push(
    shapeLayer(ind++, 'Pulse Nebula A', [ellipseGroup([980, 980], '#2a4f86', { alpha: 0.5 })], {
      pos: [360, 360, 0],
      opacity: 14,
    }),
  );
  layerList.push(
    shapeLayer(ind++, 'Pulse Nebula B', [ellipseGroup([680, 680], '#4a3580', { alpha: 0.45 })], {
      pos: [292, 420, 0],
      opacity: 10,
    }),
  );

  return {
    v: '5.13.0',
    fr: FRAME_RATE,
    ip: 0,
    op: TOTAL_FRAMES,
    w: 720,
    h: 720,
    nm: 'Pulse Core',
    ddd: 0,
    assets: [],
    layers: layerList,
    markers: [],
  };
}

/** 一个内置示例：id 用于路由，name 显示在顶部数据源按钮上。 */
export interface SampleDefinition {
  /** 数据源 id（也用于 `file:` / `demo:` 前缀之外的内置来源匹配）。 */
  id: string;
  /** 中文显示名。 */
  name: string;
  /** 生成的 Lottie 数据。 */
  data: LottieData;
}

/** 全部内置示例；在模块加载时一次性生成。 */
export const SAMPLES: SampleDefinition[] = [
  { id: 'orbit', name: '星轨', data: createOrbitSample() },
  { id: 'pulse', name: '脉冲', data: createPulseSample() },
];
