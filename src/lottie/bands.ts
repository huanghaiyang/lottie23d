// 图层分带：把 Lottie 的「前 → 后」图层序列切成若干深度平面，
// 供左侧多实例渲染，也供右侧按层计算 Z 深度。
import type { LottieData, LottieLayer } from './types';

/**
 * 一个深度分层。
 *
 * Lottie 的 layers 数组是「前 → 后」排列的（索引 0 最靠前），
 * 因此可以直接按顺序把图层分配进若干深度平面。
 */
export interface BandSpec {
  /** 0 表示最靠前的一层 */
  index: number;
  /** 仅包含该层图层（以及必要的父级链）的 Lottie 数据 */
  data: LottieData;
  /** 实际归入该层的图层数量（不含为补齐父级而引入的图层） */
  layers: number;
  /** 1 = 最前，0 = 最后，供 3D 场景换算 Z 轴位置 */
  depth: number;
}

/** 深拷贝，避免多个 Lottie 实例共享同一份可变数据 */
function clone<T>(value: T): T {
  if (typeof structuredClone === 'function') return structuredClone(value);
  return JSON.parse(JSON.stringify(value)) as T;
}

/**
 * 把一份 Lottie 数据按图层顺序拆成多个深度分层。
 *
 * 处理了两个容易出问题的细节：
 * 1. 轨道遮罩（tt）依赖紧邻其上的图层，二者必须留在同一个分层里；
 * 2. 带 parent 的图层需要父级链参与变换计算，因此即使父层属于别的分层，
 *    也要一并复制进当前分层（父层通常是空对象层，不会产生重复画面）。
 *
 * @param source 原始 Lottie 数据
 * @param requested 期望的分层数（会被 clamp 到 1 … 图层数）
 * @returns 各分层的数据副本与深度（depth 为 1 → 0，1 最靠前）
 */
export function splitIntoBands(source: LottieData, requested: number): BandSpec[] {
  const layers = source.layers ?? [];
  if (layers.length === 0) return [];

  // 分层数不能超过图层数，否则会出现空的深度平面。
  const count = Math.max(1, Math.min(Math.floor(requested), layers.length));
  if (count === 1) {
    return [{ index: 0, data: clone(source), layers: layers.length, depth: 0.5 }];
  }

  // 按原始层序均分到各分层（前 → 后）
  const bandOf = layers.map((_, i) => Math.floor((i * count) / layers.length));

  // 遮罩层与被遮罩层合并到同一分层
  for (let i = 1; i < layers.length; i++) {
    if (layers[i].tt) {
      const b = Math.min(bandOf[i], bandOf[i - 1]);
      bandOf[i] = b;
      bandOf[i - 1] = b;
    }
  }

  const byInd = new Map<number, number>();
  layers.forEach((layer, i) => byInd.set(layer.ind, i));

  const bands: BandSpec[] = [];

  for (let band = 0; band < count; band++) {
    const include = new Set<number>();

    // 递归向上收集 parent 链；guard 防止畸形数据里的父子环导致死循环。
    const addAncestors = (index: number) => {
      const guard = new Set<number>([index]);
      let parentInd = layers[index]?.parent;
      while (typeof parentInd === 'number') {
        const parentIndex = byInd.get(parentInd);
        if (parentIndex === undefined || guard.has(parentIndex)) break;
        guard.add(parentIndex);
        include.add(parentIndex);
        parentInd = layers[parentIndex]?.parent;
      }
    };

    let owned = 0;
    for (let i = 0; i < layers.length; i++) {
      if (bandOf[i] !== band) continue;
      include.add(i);
      owned++;
      addAncestors(i);
    }

    if (include.size === 0) continue;

    // 保持原始层序，避免打乱前后关系
    const indices = [...include].sort((a, b) => a - b);
    const { layers: _allLayers, ...rest } = source;
    void _allLayers;

    const data: LottieData = {
      ...clone(rest),
      layers: indices.map((i) => clone(layers[i]) as LottieLayer),
    };

    bands.push({
      index: band,
      data,
      layers: owned,
      depth: 1 - band / (count - 1),
    });
  }

  return bands;
}
