import lottie, { type AnimationItem } from 'lottie-web';
import { SAMPLES } from './samples';
import { Stage } from './scene/stage';
import { loadConverter } from '../sdk/index.mjs';
import type { Converter, Model } from '../sdk/index.mjs';
import type { LottieData } from './lottie/types';

function must<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`缺少元素 #${id}`);
  }
  return element as T;
}

const lottieHost = must<HTMLDivElement>('lottie-host');
const threeHost = must<HTMLDivElement>('three-host');
const leftMeta = must<HTMLSpanElement>('left-meta');
const rightMeta = must<HTMLSpanElement>('right-meta');
const statConversion = must<HTMLSpanElement>('stat-conversion');
const statFrames = must<HTMLSpanElement>('stat-frames');
const statGeometry = must<HTMLSpanElement>('stat-geometry');
const statSize = must<HTMLSpanElement>('stat-size');
const fpsReadout = must<HTMLSpanElement>('fps-readout');
const frameLabel = must<HTMLSpanElement>('frame-label');
const timeline = must<HTMLInputElement>('timeline');
const playToggle = must<HTMLButtonElement>('play-toggle');
const playLabel = must<HTMLSpanElement>('play-label');
const speedInput = must<HTMLInputElement>('speed');
const speedLabel = must<HTMLSpanElement>('speed-label');
const depthInput = must<HTMLInputElement>('depth');
const depthLabel = must<HTMLSpanElement>('depth-label');
const thicknessInput = must<HTMLInputElement>('thickness');
const thicknessLabel = must<HTMLSpanElement>('thickness-label');
const autorotateInput = must<HTMLInputElement>('autorotate');
const resetViewButton = must<HTMLButtonElement>('reset-view');
const pickFileButton = must<HTMLButtonElement>('pick-file');
const fileInput = must<HTMLInputElement>('file-input');
const toast = must<HTMLDivElement>('toast');
const dropOverlay = must<HTMLDivElement>('drop-overlay');

const stage = new Stage(threeHost);

let converter: Converter | null = null;
let animation: AnimationItem | null = null;
let model: Model | null = null;
let duration = 1;
let ip = 0;
let op = 0;
let frameRate = 30;
let time = 0;
let playing = true;
let speed = 1;
let draggingTimeline = false;
let resumeAfterDrag = false;
let toastTimer = 0;

function showToast(message: string): void {
  toast.textContent = message;
  toast.classList.add('is-visible');
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toast.classList.remove('is-visible'), 3200);
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${(bytes / 1024).toFixed(0)} KB`;
}

function setPlaying(next: boolean): void {
  playing = next;
  playLabel.textContent = playing ? '暂停' : '播放';
  playToggle.dataset.playing = String(playing);
}

function disposeAnimation(): void {
  if (animation) {
    animation.destroy();
    animation = null;
  }
  lottieHost.replaceChildren();
}

function createAnimation(data: LottieData): AnimationItem {
  disposeAnimation();
  return lottie.loadAnimation({
    container: lottieHost,
    renderer: 'canvas',
    loop: false,
    autoplay: false,
    animationData: data,
    rendererSettings: {
      clearCanvas: true,
      preserveAspectRatio: 'xMidYMid meet',
      dpr: Math.min(1.5, Math.max(1, window.devicePixelRatio || 1)),
    },
  });
}

let fpsAccumulator = 0;
let fpsWindowStart = 0;
let lastTick = 0;
let lastSceneHud = 0;

function syncScene(forceHud = false): void {
  if (model == null) {
    return;
  }
  const ratio = duration > 0 ? Math.min(1, Math.max(0, time / duration)) : 0;
  if (animation) {
    const lottieFrame = Math.min(op - 0.01, ip + ratio * (op - ip));
    animation.goToAndStop(lottieFrame, true);
  }
  const frameIndex = Math.round(ratio * (model.frameCount - 1));
  stage.setFrame(frameIndex);

  // Keep DOM writes off the hot path: refresh the readouts ~8 times a second.
  const now = performance.now();
  if (forceHud || now - lastSceneHud > 120) {
    lastSceneHud = now;
    if (!draggingTimeline) {
      timeline.value = String(Math.round(ratio * 1000));
    }
    frameLabel.textContent = `${String(frameIndex + 1).padStart(3, '0')} / ${model.frameCount}`;
  }
}

function updateSourceButtons(activeId: string): void {
  document.querySelectorAll<HTMLButtonElement>('[data-source]').forEach((button) => {
    button.classList.toggle('is-active', button.dataset.source === activeId);
  });
}

async function loadDocument(data: LottieData, id: string): Promise<void> {
  if (!converter) {
    showToast('转换器尚未就绪');
    return;
  }

  const started = performance.now();
  let converted: Model;
  try {
    converted = converter.convert(data);
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error));
    return;
  }
  const elapsed = performance.now() - started;

  model = converted;
  frameRate = Math.max(1, Number(data.fr) || converted.frameRate);
  ip = Number.isFinite(data.ip) ? Number(data.ip) : 0;
  op = Number.isFinite(data.op) && Number(data.op) > ip ? Number(data.op) : ip + frameRate * 3;
  if (op <= ip) {
    op = ip + frameRate * 3;
  }
  duration = Math.max(0.001, (op - ip) / frameRate);

  const created = createAnimation(data);
  animation = created;

  stage.setModel(converted);

  time = 0;
  syncScene(true);

  leftMeta.textContent = `${Number(data.w) || converted.width} × ${Number(data.h) || converted.height} · ${frameRate.toFixed(0)} fps · ${(op - ip).toFixed(0)} 帧源`;
  rightMeta.textContent = `${converted.meshCount} 网格 · ${converted.totalVertices.toLocaleString()} 顶点 · ${(converted.totalIndices / 3).toLocaleString()} 三角形`;
  statConversion.textContent = `转换 ${elapsed.toFixed(1)} ms`;
  statFrames.textContent = `${converted.frameCount} 烘焙帧 · ${converted.sampleFps.toFixed(0)} fps`;
  statGeometry.textContent = `${converted.meshCount} 网格 · ${(converted.totalIndices / 3).toLocaleString()} 三角形`;
  statSize.textContent = formatBytes(converted.totalSize);
  document.body.dataset.ready = 'true';
  updateSourceButtons(id);
  setPlaying(true);
}

async function loadSample(id: string): Promise<void> {
  const sample = SAMPLES.find((item) => item.id === id);
  if (!sample) {
    return;
  }
  await loadDocument(sample.data, id);
}

async function loadDemo(name: string): Promise<void> {
  const url = new URL(`animations/${name}.json`, document.baseURI);
  try {
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`无法加载示例（HTTP ${response.status}）`);
    }
    const data = (await response.json()) as LottieData;
    await loadDocument(data, `demo:${name}`);
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error));
  }
}

async function loadFile(file: File): Promise<void> {
  try {
    const text = await file.text();
    const data = JSON.parse(text) as LottieData;
    await loadDocument(data, `file:${file.name}`);
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error));
  }
}

function bindInputs(): void {
  playToggle.addEventListener('click', () => setPlaying(!playing));

  timeline.addEventListener('pointerdown', () => {
    draggingTimeline = true;
    resumeAfterDrag = playing;
    setPlaying(false);
  });
  const finishDrag = (): void => {
    if (!draggingTimeline) {
      return;
    }
    draggingTimeline = false;
    if (resumeAfterDrag) {
      setPlaying(true);
    }
  };
  timeline.addEventListener('pointerup', finishDrag);
  timeline.addEventListener('pointercancel', finishDrag);
  timeline.addEventListener('input', () => {
    time = (Number(timeline.value) / 1000) * duration;
    syncScene(true);
  });

  speedInput.addEventListener('input', () => {
    speed = Number(speedInput.value) / 100;
    speedLabel.textContent = `${speed.toFixed(2)}×`;
  });

  depthInput.addEventListener('input', () => {
    const amount = Number(depthInput.value) / 100;
    depthLabel.textContent = amount.toFixed(2);
    stage.setDepthScale(amount);
  });

  thicknessInput.addEventListener('input', () => {
    const amount = Number(thicknessInput.value) / 100;
    thicknessLabel.textContent = `${amount.toFixed(2)}×`;
    stage.setThicknessScale(amount);
  });

  autorotateInput.addEventListener('change', () => {
    stage.setAutoRotate(autorotateInput.checked);
  });

  resetViewButton.addEventListener('click', () => stage.resetView());

  pickFileButton.addEventListener('click', () => fileInput.click());
  fileInput.addEventListener('change', () => {
    const file = fileInput.files?.[0];
    if (file) {
      void loadFile(file);
    }
    fileInput.value = '';
  });

  document.querySelectorAll<HTMLButtonElement>('[data-source]').forEach((button) => {
    button.addEventListener('click', () => {
      const source = button.dataset.source ?? '';
      if (source.startsWith('demo:')) {
        void loadDemo(source.slice(5));
      } else {
        void loadSample(source);
      }
    });
  });

  window.addEventListener('keydown', (event) => {
    if (event.code === 'Space' && event.target === document.body) {
      event.preventDefault();
      setPlaying(!playing);
    }
  });

  let dragDepth = 0;
  window.addEventListener('dragenter', (event) => {
    event.preventDefault();
    dragDepth += 1;
    dropOverlay.classList.add('is-visible');
  });
  window.addEventListener('dragover', (event) => event.preventDefault());
  window.addEventListener('dragleave', () => {
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) {
      dropOverlay.classList.remove('is-visible');
    }
  });
  window.addEventListener('drop', (event) => {
    event.preventDefault();
    dragDepth = 0;
    dropOverlay.classList.remove('is-visible');
    const file = event.dataTransfer?.files?.[0];
    if (file) {
      void loadFile(file);
    }
  });
}

function updateHud(now: number): void {
  fpsAccumulator += 1;
  if (now - fpsWindowStart >= 500) {
    const fps = (fpsAccumulator * 1000) / (now - fpsWindowStart);
    fpsReadout.textContent = `${fps.toFixed(0)} FPS`;
    stage.adaptQuality(fps);
    fpsAccumulator = 0;
    fpsWindowStart = now;
  }
}

function tick(now: number): void {
  const delta = lastTick === 0 ? 0 : Math.min(0.1, (now - lastTick) / 1000);
  lastTick = now;

  if (playing && duration > 0) {
    time += delta * speed;
    if (time >= duration) {
      time %= duration;
    }
    syncScene();
  }
  stage.render();
  updateHud(now);
  requestAnimationFrame(tick);
}

async function boot(): Promise<void> {
  bindInputs();
  setPlaying(true);
  requestAnimationFrame(tick);

  try {
    converter = await loadConverter();
  } catch (error) {
    showToast(`Rust 转换器加载失败：${error instanceof Error ? error.message : String(error)}`);
    return;
  }

  await loadSample('orbit');
}

void boot();
