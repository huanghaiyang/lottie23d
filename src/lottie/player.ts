// 左侧原画面板：把 Lottie 按深度拆成多层，每层一个 lottie-web canvas 实例。
// 数据流：splitIntoBands 拆分 → 各自 loadAnimation(canvas) → seek() 统一驱动；
// 产出的 canvas 由上层旋转成 CanvasTexture，贴到对应的 3D 薄板上。
import lottie, { type AnimationItem } from 'lottie-web';
import { splitIntoBands } from './bands';
import type { LottieData } from './types';

/** 一个深度分层渲染出的画布 */
export interface BandSurface {
  /** 0 = 最前 */
  index: number;
  /** 1 = 最前，0 = 最后 */
  depth: number;
  /** 该分层包含的图层数量 */
  layers: number;
  /** Lottie canvas 渲染器输出的画布，可直接用作 CanvasTexture */
  canvas: HTMLCanvasElement;
}

/**
 * 用多个 lottie-web 实例分别渲染每个深度分层。
 *
 * 所有实例共用同一条时间轴：外部调用 seek(frame) 时，
 * 每个实例都被定位到同一帧，从而保证分层之间完全同步。
 */
export class LottieBands {
  readonly width: number;
  readonly height: number;
  readonly frameRate: number;
  readonly firstFrame: number;
  readonly lastFrame: number;

  private readonly host: HTMLElement;
  private readonly instances: AnimationItem[] = [];
  private readonly holders: HTMLDivElement[] = [];
  private readonly bandSurfaces: BandSurface[] = [];
  private currentFrame = Number.NaN;

  /**
   * @param host 承载各分层 holder 的容器（holder 绝对定位后互相叠放）
   * @param data 原始 Lottie 数据
   * @param bandCount 期望的深度分层数（实际会被 clamp 到图层数）
   */
  constructor(host: HTMLElement, data: LottieData, bandCount: number) {
    this.host = host;
    this.width = Math.max(1, Number(data.w) || 1);
    this.height = Math.max(1, Number(data.h) || 1);
    this.frameRate = Number(data.fr) || 30;
    this.firstFrame = Number.isFinite(data.ip) ? data.ip : 0;
    this.lastFrame = Number.isFinite(data.op) ? data.op : this.firstFrame + this.frameRate * 3;

    const totalSide = Math.max(this.width, this.height);
    // 限制每张纹理的最大边长，避免多层高分辨率画布带来过高的绘制与上传开销
    const maxTextureSide = 768;
    // 画布像素比按“最长边不超过 768px”反推，并夹在 0.6–1.5 之间，
    // 兼顾清晰度与多层纹理的上传/显存开销。
    const dpr = Math.max(0.6, Math.min(1.5, maxTextureSide / totalSide));

    const specs = splitIntoBands(data, bandCount);

    for (const spec of specs) {
      const holder = document.createElement('div');
      holder.style.position = 'absolute';
      holder.style.left = '0';
      holder.style.top = '0';
      holder.style.width = `${this.width}px`;
      holder.style.height = `${this.height}px`;
      this.host.appendChild(holder);
      this.holders.push(holder);

      const animation = lottie.loadAnimation({
        container: holder,
        renderer: 'canvas',
        loop: false,
        autoplay: false,
        animationData: spec.data,
        rendererSettings: {
          clearCanvas: true,
          preserveAspectRatio: 'xMidYMid meet',
          dpr,
        },
      });

      const canvas = holder.querySelector('canvas');
      if (!canvas) {
        animation.destroy();
        holder.remove();
        throw new Error('Lottie 画布初始化失败');
      }

      this.instances.push(animation);
      this.bandSurfaces.push({
        index: spec.index,
        depth: spec.depth,
        layers: spec.layers,
        canvas,
      });
    }
  }

  get surfaces(): BandSurface[] {
    return this.bandSurfaces;
  }

  /** 动画总时长（秒） */
  get duration(): number {
    return Math.max(0.001, (this.lastFrame - this.firstFrame) / this.frameRate);
  }

  /**
   * 把全部分层定位到同一帧（帧单位，非秒）。
   * @returns 帧是否发生变化（用于决定是否需要更新纹理）
   */
  seek(frame: number): boolean {
    // 先夹到 [ip, op]，再与上次帧比较；1e-4 阈值避免浮点噪声导致每帧重复重绘。
    const clamped = Math.min(this.lastFrame, Math.max(this.firstFrame, frame));
    if (Number.isNaN(this.currentFrame) || Math.abs(clamped - this.currentFrame) > 1e-4) {
      this.currentFrame = clamped;
      for (const animation of this.instances) {
        animation.goToAndStop(clamped, true);
      }
      return true;
    }
    return false;
  }

  /** 销毁所有 lottie 实例并移除 holder，避免重复加载时泄漏 canvas。 */
  destroy(): void {
    for (const animation of this.instances) {
      animation.destroy();
    }
    this.instances.length = 0;
    for (const holder of this.holders) {
      holder.remove();
    }
    this.holders.length = 0;
    this.bandSurfaces.length = 0;
  }
}
