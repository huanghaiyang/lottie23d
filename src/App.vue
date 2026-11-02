<script setup lang="ts">
import lottie, { type AnimationItem } from 'lottie-web';
import { onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue';
import { loadConverter } from '../sdk/index.mjs';
import type { Converter, ConvertOptions, Model } from '../sdk/index.mjs';
import TopBar from './components/TopBar.vue';
import TransportBar from './components/TransportBar.vue';
import InspectorPanel from './components/InspectorPanel.vue';
import type { LottieData } from './lottie/types';
import { SAMPLES } from './samples';
import { AXIS_LABELS, Stage, type StageNode, type ViewAxis } from './scene/stage';

const SOURCES = [
  { id: 'orbit', label: '星轨' },
  { id: 'pulse', label: '脉冲' },
  { id: 'demo:starfish', label: '海星' },
  { id: 'demo:lights', label: '彩灯' },
  { id: 'demo:dalek', label: 'Dalek' },
  { id: 'pick', label: '导入 JSON', accent: true },
];

const lottieHost = ref<HTMLDivElement | null>(null);
const threeHost = ref<HTMLDivElement | null>(null);
const fileInput = ref<HTMLInputElement | null>(null);

const ready = ref(false);
const playing = ref(true);
const speed = ref(1);
const depth = ref(1);
const thickness = ref(1);
const edgeRatio = ref(0);
const sampleFps = ref(30);
const autoRotate = ref(false);
const axisLock = ref(false);
const activeAxis = ref<ViewAxis | null>(null);
const nodes = ref<StageNode[]>([]);
const selectedIds = ref<number[]>([]);
const softGlow = ref(false);
const nodeGlow = reactive<Record<number, boolean>>({});
let selectionAnchor = -1;
const progress = ref(0);
const frameLabel = ref('000 / 000');
const fps = ref(0);
const activeSource = ref('');
const leftMeta = ref('--');
const rightMeta = ref('--');
const toastText = ref('');
const toastVisible = ref(false);
const dropVisible = ref(false);
const stats = reactive({
  conversion: '等待转换',
  frames: '-- 帧',
  geometry: '-- 网格',
  size: '--',
});

let stage: Stage | null = null;
let converter: Converter | null = null;
let animation: AnimationItem | null = null;
let model: Model | null = null;
let currentData: LottieData | null = null;
let duration = 1;
let ip = 0;
let op = 0;
let frameRate = 30;
let time = 0;
let lastTick = 0;
let rafId = 0;
let toastTimer = 0;
let lastSceneHud = 0;
let draggingTimeline = false;
let resumeAfterDrag = false;
let fpsAccumulator = 0;
let fpsWindowStart = 0;
let dragDepth = 0;
let reconvertTimer = 0;
let convertKey = '';

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${(bytes / 1024).toFixed(0)} KB`;
}

function showToast(message: string): void {
  toastText.value = message;
  toastVisible.value = true;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toastVisible.value = false;
  }, 3200);
}

function setPlaying(next: boolean): void {
  playing.value = next;
}

function togglePlay(): void {
  setPlaying(!playing.value);
}

function disposeAnimation(): void {
  animation?.destroy();
  animation = null;
  lottieHost.value?.replaceChildren();
}

function createAnimation(data: LottieData): AnimationItem {
  disposeAnimation();
  return lottie.loadAnimation({
    container: lottieHost.value as HTMLDivElement,
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

function syncScene(forceHud = false): void {
  if (model == null) {
    return;
  }
  const ratio = duration > 0 ? Math.min(1, Math.max(0, time / duration)) : 0;
  if (animation) {
    animation.goToAndStop(Math.min(op - 0.01, ip + ratio * (op - ip)), true);
  }
  const frameIndex = Math.round(ratio * (model.frameCount - 1));
  stage?.setFrame(frameIndex);

  // Keep Vue updates off the hot path: refresh the readouts ~8 times a second.
  const now = performance.now();
  if (forceHud || now - lastSceneHud > 120) {
    lastSceneHud = now;
    if (!draggingTimeline) {
      progress.value = Math.round(ratio * 1000);
    }
    frameLabel.value = `${String(frameIndex + 1).padStart(3, '0')} / ${model.frameCount}`;
  }
}

function onSeek(value: number): void {
  if (duration <= 0) {
    return;
  }
  time = (value / 1000) * duration;
  progress.value = value;
  syncScene(true);
}

function onDragStart(): void {
  draggingTimeline = true;
  resumeAfterDrag = playing.value;
  setPlaying(false);
}

function onDragEnd(): void {
  if (!draggingTimeline) {
    return;
  }
  draggingTimeline = false;
  if (resumeAfterDrag) {
    setPlaying(true);
  }
}

function resetView(): void {
  activeAxis.value = null;
  axisLock.value = false;
  stage?.resetView();
}

/** Clicking a gizmo axis snaps the camera and locks the orbit for inspection. */
function onAxisPick(axis: ViewAxis): void {
  activeAxis.value = axis;
  autoRotate.value = false;
  axisLock.value = true;
  stage?.setAutoRotate(false);
  stage?.setRotationLock(true);
}

function syncNodes(): void {
  nodes.value = model?.nodes ? [...model.nodes] : [];
  selectedIds.value = [];
  selectionAnchor = -1;
  for (const key of Object.keys(nodeGlow)) {
    delete nodeGlow[Number(key)];
  }
  stage?.setSelection([]);
}

/**
 * Plain click selects one node, Shift extends a range from the anchor and
 * Alt (or Ctrl) toggles the node inside a multi-selection.
 */
function onNodeSelect(
  id: number,
  modifiers: { shift: boolean; alt: boolean; ctrl: boolean },
): void {
  const list = nodes.value;
  if (modifiers.alt || modifiers.ctrl) {
    const selected = new Set(selectedIds.value);
    if (selected.has(id)) {
      selected.delete(id);
    } else {
      selected.add(id);
    }
    selectedIds.value = list.filter((node) => selected.has(node.id)).map((node) => node.id);
    selectionAnchor = id;
  } else if (modifiers.shift && selectionAnchor >= 0) {
    const from = list.findIndex((node) => node.id === selectionAnchor);
    const to = list.findIndex((node) => node.id === id);
    if (from >= 0 && to >= 0) {
      const start = Math.min(from, to);
      const end = Math.max(from, to);
      selectedIds.value = list.slice(start, end + 1).map((node) => node.id);
    }
  } else {
    selectedIds.value = [id];
    selectionAnchor = id;
  }
  stage?.setSelection(selectedIds.value);
}

/** Per-node glow flag; nodes default to enabled once the master switch is on. */
function toggleNodeGlow(id: number): void {
  const next = nodeGlow[id] === false;
  nodeGlow[id] = next;
  stage?.setNodeGlow(id, next);
}

function convertOptions(): ConvertOptions {
  const options: ConvertOptions = { fps: sampleFps.value };
  const half = model?.meshes[0]?.halfThickness ?? 0;
  // Edge rounding is expressed as a ratio of the baked half-thickness.
  if (edgeRatio.value > 0 && half > 0) {
    options.edge_radius = (edgeRatio.value / 100) * half;
  }
  return options;
}

function optionsKey(): string {
  return `${sampleFps.value}|${edgeRatio.value}`;
}

function applyStats(converted: Model, elapsed: number): void {
  rightMeta.value = `${converted.meshCount} 网格 · ${converted.totalVertices.toLocaleString()} 顶点 · ${(converted.totalIndices / 3).toLocaleString()} 三角形`;
  stats.conversion = `转换 ${elapsed.toFixed(1)} ms`;
  stats.frames = `${converted.frameCount} 烘焙帧 · ${converted.sampleFps.toFixed(0)} fps`;
  stats.geometry = `${converted.meshCount} 网格 · ${(converted.totalIndices / 3).toLocaleString()} 三角形`;
  stats.size = formatBytes(converted.totalSize);
}

/** Rebuild the mesh with the current conversion options, keeping playback time. */
function reconvert(): void {
  if (converter == null || currentData == null) {
    return;
  }
  const key = optionsKey();
  if (key === convertKey) {
    return;
  }
  const ratio = duration > 0 ? time / duration : 0;
  const started = performance.now();
  let converted: Model;
  try {
    converted = converter.convert(currentData, convertOptions());
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error));
    return;
  }
  const elapsed = performance.now() - started;
  convertKey = key;
  model = converted;
  stage?.setModel(converted);
  syncNodes();
  time = ratio * duration;
  syncScene(true);
  applyStats(converted, elapsed);
}

function scheduleReconvert(): void {
  if (currentData == null) {
    return;
  }
  window.clearTimeout(reconvertTimer);
  reconvertTimer = window.setTimeout(reconvert, 90);
}

async function loadDocument(data: LottieData, id: string): Promise<void> {
  if (!converter) {
    showToast('转换器尚未就绪');
    return;
  }

  currentData = data;
  frameRate = Math.max(1, Number(data.fr) || 30);
  ip = Number.isFinite(data.ip) ? Number(data.ip) : 0;
  op = Number.isFinite(data.op) && Number(data.op) > ip ? Number(data.op) : ip + frameRate * 3;
  if (op <= ip) {
    op = ip + frameRate * 3;
  }
  duration = Math.max(0.001, (op - ip) / frameRate);
  sampleFps.value = Math.min(120, Math.max(6, Math.round(frameRate)));
  edgeRatio.value = 0;

  const started = performance.now();
  let converted: Model;
  try {
    converted = converter.convert(data, convertOptions());
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error));
    return;
  }
  const elapsed = performance.now() - started;
  convertKey = optionsKey();

  model = converted;
  animation = createAnimation(data);
  stage?.setModel(converted);
  syncNodes();

  time = 0;
  syncScene(true);

  leftMeta.value = `${Number(data.w) || converted.width} × ${Number(data.h) || converted.height} · ${frameRate.toFixed(0)} fps · ${(op - ip).toFixed(0)} 帧源`;
  applyStats(converted, elapsed);
  activeSource.value = id;
  ready.value = true;
  setPlaying(true);
}

async function loadSample(id: string): Promise<void> {
  const sample = SAMPLES.find((item) => item.id === id);
  if (sample) {
    await loadDocument(sample.data, id);
  }
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

function onSelectSource(id: string): void {
  if (id.startsWith('demo:')) {
    void loadDemo(id.slice(5));
  } else {
    void loadSample(id);
  }
}

function pickFile(): void {
  fileInput.value?.click();
}

function onFileChange(event: Event): void {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (file) {
    void loadFile(file);
  }
  input.value = '';
}

function onKeyDown(event: KeyboardEvent): void {
  if (event.code === 'Space' && event.target === document.body) {
    event.preventDefault();
    togglePlay();
  }
}

function onDragEnter(event: DragEvent): void {
  event.preventDefault();
  dragDepth += 1;
  dropVisible.value = true;
}

function onDragOver(event: DragEvent): void {
  event.preventDefault();
}

function onDragLeave(): void {
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) {
    dropVisible.value = false;
  }
}

function onDrop(event: DragEvent): void {
  event.preventDefault();
  dragDepth = 0;
  dropVisible.value = false;
  const file = event.dataTransfer?.files?.[0];
  if (file) {
    void loadFile(file);
  }
}

function updateHud(now: number): void {
  fpsAccumulator += 1;
  if (now - fpsWindowStart >= 500) {
    const value = (fpsAccumulator * 1000) / (now - fpsWindowStart);
    fps.value = Math.round(value);
    stage?.adaptQuality(value);
    fpsAccumulator = 0;
    fpsWindowStart = now;
  }
}

function tick(now: number): void {
  const delta = lastTick === 0 ? 0 : Math.min(0.1, (now - lastTick) / 1000);
  lastTick = now;

  if (playing.value && duration > 0) {
    time += delta * speed.value;
    if (time >= duration) {
      time %= duration;
    }
    syncScene();
  }
  stage?.render();
  updateHud(now);
  rafId = requestAnimationFrame(tick);
}

async function boot(): Promise<void> {
  try {
    converter = await loadConverter();
  } catch (error) {
    showToast(`Rust 转换器加载失败：${error instanceof Error ? error.message : String(error)}`);
    return;
  }
  await loadSample('orbit');
}

watch(depth, (value) => stage?.setDepthScale(value));
watch(thickness, (value) => stage?.setThicknessScale(value));
watch(autoRotate, (value) => {
  if (value && axisLock.value) {
    axisLock.value = false;
    // Unlock first: setAutoRotate ignores the request while still locked.
    stage?.setRotationLock(false);
  }
  stage?.setAutoRotate(value);
});
watch(axisLock, (value) => {
  if (value) {
    autoRotate.value = false;
  }
  stage?.setRotationLock(value);
});
watch(edgeRatio, scheduleReconvert);
watch(sampleFps, scheduleReconvert);
watch(softGlow, (value) => stage?.setSoftGlow(value));

onMounted(() => {
  stage = new Stage(threeHost.value as HTMLDivElement, { onAxisPick });
  stage.setDepthScale(depth.value);
  stage.setThicknessScale(thickness.value);
  stage.setAutoRotate(autoRotate.value);
  stage.setSoftGlow(softGlow.value);

  rafId = requestAnimationFrame(tick);
  window.addEventListener('keydown', onKeyDown);
  window.addEventListener('dragenter', onDragEnter);
  window.addEventListener('dragover', onDragOver);
  window.addEventListener('dragleave', onDragLeave);
  window.addEventListener('drop', onDrop);

  void boot();
});

onBeforeUnmount(() => {
  cancelAnimationFrame(rafId);
  window.removeEventListener('keydown', onKeyDown);
  window.removeEventListener('dragenter', onDragEnter);
  window.removeEventListener('dragover', onDragOver);
  window.removeEventListener('dragleave', onDragLeave);
  window.removeEventListener('drop', onDrop);
  window.clearTimeout(toastTimer);
  window.clearTimeout(reconvertTimer);
  disposeAnimation();
  stage?.dispose();
  stage = null;
});
</script>

<template>
  <div class="app">
    <TopBar
      :sources="SOURCES"
      :active-source="activeSource"
      :conversion="stats.conversion"
      :frames="stats.frames"
      :geometry="stats.geometry"
      :size="stats.size"
      @select="onSelectSource"
      @pick="pickFile"
    />

    <main class="workspace">
      <section class="pane">
        <div class="pane-head">
          <div class="pane-title">
            <span class="dot dot-lottie"></span>
            <h2>Lottie Web</h2>
          </div>
          <span class="pane-meta">{{ leftMeta }}</span>
        </div>
        <div class="canvas-wrap canvas-lottie">
          <div ref="lottieHost" class="lottie-mount"></div>
          <span class="canvas-hint" :class="{ 'is-hidden': ready }">加载中…</span>
        </div>
        <span class="pane-badge">2D 原画</span>
      </section>

      <section class="pane">
        <div class="pane-head">
          <div class="pane-title">
            <span class="dot dot-three"></span>
            <h2>Three.js</h2>
          </div>
          <span class="pane-meta">{{ rightMeta }}</span>
        </div>
        <div ref="threeHost" class="canvas-wrap canvas-three">
          <InspectorPanel
            v-model:thickness="thickness"
            v-model:depth="depth"
            v-model:edge-ratio="edgeRatio"
            v-model:sample-fps="sampleFps"
            v-model:auto-rotate="autoRotate"
            v-model:axis-lock="axisLock"
            v-model:soft-glow="softGlow"
            :nodes="nodes"
            :selected="selectedIds"
            :glow="nodeGlow"
            @select-node="onNodeSelect"
            @toggle-node-glow="toggleNodeGlow"
          />
          <span class="canvas-hint">
            {{
              activeAxis
                ? `轴向 ${AXIS_LABELS[activeAxis]}${axisLock ? ' · 已锁定' : ''} · 滚轮缩放`
                : axisLock
                  ? '视角已锁定 · 滚轮缩放 · 点击轴向切换'
                  : '拖拽环绕 · 滚轮缩放 · 点击轴向切换'
            }}
          </span>
        </div>
        <span class="pane-badge">Rust 生成网格</span>
      </section>
    </main>

    <TransportBar
      v-model:speed="speed"
      :playing="playing"
      :progress="progress"
      :frame-label="frameLabel"
      :fps="fps"
      @toggle-play="togglePlay"
      @seek="onSeek"
      @drag-start="onDragStart"
      @drag-end="onDragEnd"
      @reset-view="resetView"
    />
  </div>

  <div class="drop-overlay" :class="{ 'is-visible': dropVisible }">
    <div class="drop-card">
      <strong>释放以转换 Lottie</strong>
      <span>支持 Bodymovin 导出的 .json</span>
    </div>
  </div>

  <div class="toast" :class="{ 'is-visible': toastVisible }" role="status">
    {{ toastText }}
  </div>

  <input
    ref="fileInput"
    type="file"
    accept=".json,application/json"
    hidden
    @change="onFileChange"
  />
</template>
