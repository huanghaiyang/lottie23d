<script setup lang="ts">
import lottie, { type AnimationItem } from 'lottie-web';
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue';
import { loadConverter } from '../sdk/index.mjs';
import type { Converter, ConvertOptions, Model } from '../sdk/index.mjs';
import TopBar from './components/TopBar.vue';
import TransportBar from './components/TransportBar.vue';
import InspectorPanel from './components/InspectorPanel.vue';
import LibraryPanel from './components/LibraryPanel.vue';
import {
  deleteLibraryFile,
  listLibrary,
  readLibraryFile,
  saveLibraryFile,
  type LibraryFile,
} from './library';
import type { LottieData } from './lottie/types';
import { SAMPLES } from './samples';
import { AXIS_LABELS, Stage, type StageNode, type ViewAxis } from './scene/stage';

// 页面级数据流：
// Lottie JSON → converter.convert() 得到二进制模型 → Stage.setModel() 建立网格；
// 左侧 lottie-web 原画与右侧 three.js 由同一条 rAF 时钟驱动、按进度比例同步；
// 只有“边缘平滑 / 采样帧率”需要重转换（90ms 防抖），厚度与层深在渲染期即时生效。

/** 顶栏可切换的动画来源；`demo:` 前缀表示从 public/animations 拉取。 */
const SOURCES = [
  { id: 'orbit', label: '星轨' },
  { id: 'pulse', label: '脉冲' },
  { id: 'demo:starfish', label: '海星' },
  { id: 'demo:lights', label: '彩灯' },
  { id: 'demo:dalek', label: 'Dalek' },
  { id: 'pick', label: '导入 JSON', accent: true },
];

// ---- 挂载点 ----
const lottieHost = ref<HTMLDivElement | null>(null);
const threeHost = ref<HTMLDivElement | null>(null);
const fileInput = ref<HTMLInputElement | null>(null);

// ---- 播放与视图状态 ----
const ready = ref(false);
const playing = ref(true);
const speed = ref(1);
const depth = ref(1);
const thickness = ref(1);
const edgeRatio = ref(0);
const sampleFps = ref(30);
/** 轮廓采样间距（Lottie 像素/点，1..8）：越小曲线越平滑、顶点越多。 */
const curveSpacing = ref(3);
const autoRotate = ref(false);
const axisLock = ref(false);
const activeAxis = ref<ViewAxis | null>(null);
// ---- 节点列表与多选 ----
/** 当前模型的节点快照（按渲染顺序，id = meshes 下标）。 */
const nodes = ref<StageNode[]>([]);
/** 选中节点 id；Shift 范围选择依赖 selectionAnchor 作为锚点。 */
const selectedIds = ref<number[]>([]);
/** 总柔光开关：开启时所有节点默认发光。 */
const softGlow = ref(false);
/** 柔光强度倍率（0..2）：1 为默认，0 相当于关闭叠加。 */
const glowIntensity = ref(1);
/** 柔光半径倍率（0.2..3）：1 为默认，越大越柔和。 */
const glowRadius = ref(1);
/** 单节点柔光覆盖：true 强制发光，false 强制排除（与总开关组合）。 */
const nodeGlow = reactive<Record<number, boolean>>({});
/** 单节点显隐，缺省（undefined）视为可见。 */
const nodeVisible = reactive<Record<number, boolean>>({});
/** Shift 范围选择的起点节点 id；-1 表示无锚点。 */
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
/** 顶栏统计（转换耗时 / 帧数 / 几何规模 / 文件大小）。 */
const stats = reactive({
  conversion: '等待转换',
  frames: '-- 帧',
  geometry: '-- 网格',
  size: '--',
});

// ---- 资源库（项目根目录 data/，由开发服务器读写） ----
/** data/ 下的文件列表；服务端不可用时保持为空。 */
const libraryFiles = ref<LibraryFile[]>([]);
/** 资源库面板是否展开。 */
const libraryOpen = ref(false);
/** 资源库接口是否可用（仅 dev / preview）；不可用时隐藏入口。 */
const libraryAvailable = ref(false);
/** 上传 / 批量校验进行中。 */
const libraryBusy = ref(false);
/** 批量校验结果，key 为文件名。 */
const libraryStatus = reactive<Record<string, string>>({});

/** 当前预览文件是否来自资源库；用于面板内高亮。 */
const activeLibraryName = computed(() =>
  activeSource.value.startsWith('lib:') ? activeSource.value.slice(4) : '',
);

/**
 * 顶栏数据源：内置示例 + 外部示例 + 资源库入口。
 * 资源库入口只在接口可用时插入，位置固定在「导入 JSON」之前。
 */
const sources = computed(() => {
  const list: Array<{ id: string; label: string; accent?: boolean; match?: string }> = [
    ...SOURCES,
  ];
  if (libraryAvailable.value) {
    const count = libraryFiles.value.length;
    list.splice(list.length - 1, 0, {
      id: 'library',
      label: count > 0 ? `资源库 ${count}` : '资源库',
      match: 'lib:',
    });
  }
  return list;
});

// ---- 非响应式的运行时对象 ----
let stage: Stage | null = null;
let converter: Converter | null = null;
let animation: AnimationItem | null = null;
let model: Model | null = null;
let currentData: LottieData | null = null;
// 时间轴状态（单位：秒 / 源帧）。
let duration = 1;
let ip = 0;
let op = 0;
let frameRate = 30;
/** 当前播放时间（秒），由 rAF 时钟推进或拖动进度条直接设置。 */
let time = 0;
let lastTick = 0;
let rafId = 0;
let toastTimer = 0;
/** 上次刷新 HUD 的时间戳，用于把 Vue 更新节流到约 8 次/秒。 */
let lastSceneHud = 0;
let draggingTimeline = false;
/** 拖动进度条前是否在播放；松手后据此决定是否恢复。 */
let resumeAfterDrag = false;
let fpsAccumulator = 0;
let fpsWindowStart = 0;
/** 拖拽进入/离开计数，避免子元素抖动导致遮罩闪烁。 */
let dragDepth = 0;
/** 重转换防抖定时器与上次转换的参数键。 */
let reconvertTimer = 0;
let convertKey = '';

/** 把字节数格式化成 KB / MB，用于顶栏统计。 */
function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${(bytes / 1024).toFixed(0)} KB`;
}

/** 顶栏 / 底部提示：显示 3.2 秒，重复调用会重置计时。 */
function showToast(message: string): void {
  toastText.value = message;
  toastVisible.value = true;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toastVisible.value = false;
  }, 3200);
}

/** 统一的播放状态入口（后续可在此扩展暂停 / 恢复逻辑）。 */
function setPlaying(next: boolean): void {
  playing.value = next;
}

/** 空格键与播放按钮共用的切换逻辑。 */
function togglePlay(): void {
  setPlaying(!playing.value);
}

/** 销毁左侧 lottie-web 实例并清空容器，切换动画 / 卸载页面时调用。 */
function disposeAnimation(): void {
  animation?.destroy();
  animation = null;
  lottieHost.value?.replaceChildren();
}

/**
 * 创建左侧原画实例（canvas 渲染器，不自动播放，由外部时钟 seek）。
 * dpr 限制在 1–1.5，避免原画面板在高分屏上浪费填充率。
 */
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

/**
 * 双侧同步的核心：以归一化进度 ratio 为唯一真值，
 * 左侧 lottie-web 定位到源帧 ip + ratio * (op - ip)，
 * 右侧 three.js 定位到烘焙帧 round(ratio * (frameCount - 1))。
 * 两侧帧率与帧数不同，但都在同一进度上，因此视觉节奏一致。
 *
 * @param forceHud 为 true 时立即刷新进度 / 帧号（拖动、切换动画时需要）
 */
function syncScene(forceHud = false): void {
  if (model == null) {
    return;
  }
  const ratio = duration > 0 ? Math.min(1, Math.max(0, time / duration)) : 0;
  // 原画取 op - 0.01 作为上限，避免落在尾帧之外导致空白。
  if (animation) {
    animation.goToAndStop(Math.min(op - 0.01, ip + ratio * (op - ip)), true);
  }
  const frameIndex = Math.round(ratio * (model.frameCount - 1));
  stage?.setFrame(frameIndex);

  // 热路径上避免频繁触发 Vue 更新：HUD 最多约 8 次/秒；
  // 拖动进度条时不要反向写 progress，否则会与用户输入打架。
  const now = performance.now();
  if (forceHud || now - lastSceneHud > 120) {
    lastSceneHud = now;
    if (!draggingTimeline) {
      progress.value = Math.round(ratio * 1000);
    }
    frameLabel.value = `${String(frameIndex + 1).padStart(3, '0')} / ${model.frameCount}`;
  }
}

/** 进度条回调：value 为 0–1000 的整数，换算成时间后强制刷新两侧。 */
function onSeek(value: number): void {
  if (duration <= 0) {
    return;
  }
  time = (value / 1000) * duration;
  progress.value = value;
  syncScene(true);
}

/** 开始拖动进度条：暂停播放并记住原状态，松手后按需恢复。 */
function onDragStart(): void {
  draggingTimeline = true;
  resumeAfterDrag = playing.value;
  setPlaying(false);
}

/** 结束拖动：若拖动前在播放则恢复播放。 */
function onDragEnd(): void {
  if (!draggingTimeline) {
    return;
  }
  draggingTimeline = false;
  if (resumeAfterDrag) {
    setPlaying(true);
  }
}

/** 重置视角：同时解除轴向高亮与视角锁定。 */
function resetView(): void {
  activeAxis.value = null;
  axisLock.value = false;
  stage?.resetView();
}

/**
 * 点击 gizmo 轴向：相机已由 Stage 补间过去，这里同步 UI 状态——
 * 关闭自动环绕并锁定环绕，避免用户误拖把视角带偏。
 */
function onAxisPick(axis: ViewAxis): void {
  activeAxis.value = axis;
  autoRotate.value = false;
  axisLock.value = true;
  stage?.setAutoRotate(false);
  stage?.setRotationLock(true);
}

/** 模型切换后重建节点列表，并清空选择、柔光与显隐等逐节点临时状态。 */
function syncNodes(): void {
  nodes.value = model?.nodes ? [...model.nodes] : [];
  selectedIds.value = [];
  selectionAnchor = -1;
  // 节点 id 在新模型中会被复用，旧标记必须清掉，否则语义串到别的图层。
  for (const key of Object.keys(nodeGlow)) {
    delete nodeGlow[Number(key)];
  }
  for (const key of Object.keys(nodeVisible)) {
    delete nodeVisible[Number(key)];
  }
  stage?.setSelection([]);
}

/**
 * 节点列表的点击选择规则（与 InspectorPanel 的修饰键映射一一对应）：
 * - 单击：只选中该节点；若它本来就是唯一选中项，则再点一次取消选择；
 * - Shift：以最近一次点击的锚点做区间选择（按列表渲染顺序取连续段）；
 * - Alt / Ctrl：在已选集合中切换该节点的选中状态，可累积多选。
 * 结果统一写回 `selectedIds`，再交给 Stage 刷新线框。
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
  } else if (selectedIds.value.length === 1 && selectedIds.value[0] === id) {
    selectedIds.value = [];
    selectionAnchor = -1;
  } else {
    selectedIds.value = [id];
    selectionAnchor = id;
  }
  stage?.setSelection(selectedIds.value);
}

/**
 * 节点列表拖拽重排：把 `fromId` 插入到 `toId` 的上方或下方。
 * 节点 id 不变（显隐/柔光状态保持），只改变列表顺序与 Stage 的层深分配。
 */
function onNodeReorder(fromId: number, toId: number, place: 'before' | 'after'): void {
  const list = nodes.value;
  const fromIndex = list.findIndex((node) => node.id === fromId);
  if (fromIndex < 0 || fromId === toId) {
    return;
  }
  const next = [...list];
  const [moved] = next.splice(fromIndex, 1);
  // 源节点移除后重新定位目标，避免下标偏移。
  const target = next.findIndex((node) => node.id === toId);
  if (target < 0) {
    return;
  }
  const insert = place === 'after' ? target + 1 : target;
  next.splice(insert, 0, moved);
  // 顺序未变（例如拖到自身的相邻位置）时不做无意义的重放。
  if (next.every((node, index) => node.id === list[index].id)) {
    return;
  }
  nodes.value = next;
  // 选中集合按新顺序重排，保证 Shift 范围选择继续沿列表顺序工作。
  const selected = new Set(selectedIds.value);
  selectedIds.value = next.filter((node) => selected.has(node.id)).map((node) => node.id);
  stage?.setNodeOrder(next.map((node) => node.id));
}

/**
 * 重转换后恢复用户拖拽过的层级顺序。
 * 仅当节点 id 集合与当前完全一致时应用；节点结构变化则保留烘焙顺序。
 */
function restoreNodeOrder(ids: readonly number[]): void {
  if (ids.length !== nodes.value.length || nodes.value.length === 0) {
    return;
  }
  const byId = new Map(nodes.value.map((node) => [node.id, node]));
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
  nodes.value = next;
  stage?.setNodeOrder(next.map((node) => node.id));
}

/**
 * 单节点是否发光，与 Stage 的判定保持一致（两者必须同源，否则 UI 圆点会与实际渲染不符）：
 * - 总开关开：默认全部发光，只有显式标记为 `false` 的节点被排除；
 * - 总开关关：默认全部不发光，只有显式标记为 `true` 的节点发光。
 * Alt 点击（solo）走单独的“只留一个”分支。
 */
function nodeGlowActive(id: number): boolean {
  return softGlow.value ? nodeGlow[id] !== false : nodeGlow[id] === true;
}

/**
 * 切换单节点柔光。
 * @param id 节点 id
 * @param solo 为 true 时执行“独奏”：只保留该节点发光、其余全部关闭
 */
function toggleNodeGlow(id: number, solo: boolean): void {
  // 独奏需要逐节点写入显式标记，否则总开关开启时其它节点会被默认点亮。
  if (solo) {
    for (const node of nodes.value) {
      const enabled = node.id === id;
      nodeGlow[node.id] = enabled;
      stage?.setNodeGlow(node.id, enabled);
    }
    return;
  }
  const next = !nodeGlowActive(id);
  nodeGlow[id] = next;
  stage?.setNodeGlow(id, next);
}

/** 节点列表的“眼睛”开关：切换手动显隐；未记录过的节点默认可见。 */
function toggleNodeVisibility(id: number): void {
  const next = nodeVisible[id] === false;
  nodeVisible[id] = next;
  stage?.setNodeVisible(id, next);
}

/**
 * 把 UI 参数换算成 Rust 转换选项。
 * 只有 `edge_radius` 需要换算：滑块给的是“占半厚度的百分比”，
 * 而 ABI 期望的是像素值，所以要乘上第 0 个网格已烘焙的 `halfThickness`。
 */
function convertOptions(): ConvertOptions {
  const options: ConvertOptions = {
    fps: sampleFps.value,
    // 轮廓采样间距（像素/点）：1..8，越小越平滑，由 Rust 侧再次钳制。
    curve_spacing: curveSpacing.value,
  };
  const half = model?.meshes[0]?.halfThickness ?? 0;
  // 圆角半径以半厚度百分比表示，避免不同尺寸图层看起来圆角不一致。
  if (edgeRatio.value > 0 && half > 0) {
    options.edge_radius = (edgeRatio.value / 100) * half;
  }
  return options;
}

/** 转换选项指纹：与上一次一致说明无需重转换。 */
function optionsKey(): string {
  return `${sampleFps.value}|${edgeRatio.value}|${curveSpacing.value}`;
}

/** 把本次转换结果的统计写入各面板标签（耗时、帧数、三角形数、二进制体积）。 */
function applyStats(converted: Model, elapsed: number): void {
  rightMeta.value = `${converted.meshCount} 网格 · ${converted.totalVertices.toLocaleString()} 顶点 · ${(converted.totalIndices / 3).toLocaleString()} 三角形`;
  stats.conversion = `转换 ${elapsed.toFixed(1)} ms`;
  stats.frames = `${converted.frameCount} 烘焙帧 · ${converted.sampleFps.toFixed(0)} fps`;
  stats.geometry = `${converted.meshCount} 网格 · ${(converted.totalIndices / 3).toLocaleString()} 三角形`;
  stats.size = formatBytes(converted.totalSize);
}

/**
 * 用当前转换选项重建网格，并保持播放进度。
 * 重转换会生成全新的 Model（顶点/拓扑都可能变化），因此必须重设 Stage 模型并重建节点列表；
 * 时间按“完成度比例”映射到新时长上，避免参数一改画面就跳回开头。
 */
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
  const previousOrder = nodes.value.map((node) => node.id);
  convertKey = key;
  model = converted;
  stage?.setModel(converted);
  syncNodes();
  // 重转换不应丢失用户拖拽出的层级顺序；节点结构一致时重新套用。
  restoreNodeOrder(previousOrder);
  // 先同步模型再写时间，最后强制刷新一帧让新几何立即上屏。
  time = ratio * duration;
  syncScene(true);
  applyStats(converted, elapsed);
}

/** 合并连续的参数拖动：90ms 内多次改动只触发一次重转换，避免拖动滑块时反复编译。 */
function scheduleReconvert(): void {
  if (currentData == null) {
    return;
  }
  window.clearTimeout(reconvertTimer);
  reconvertTimer = window.setTimeout(reconvert, 90);
}

/**
 * 载入一份 Lottie 文档：先确定时间模型，再调 Rust 转换，最后重建动画与场景。
 * @param data 原始 Lottie JSON（Bodymovin 导出）
 * @param id 数据源标识（示例 id、`demo:名称` 或 `file:文件名`），用于高亮顶部来源
 */
async function loadDocument(data: LottieData, id: string): Promise<void> {
  if (!converter) {
    showToast('转换器尚未就绪');
    return;
  }

  currentData = data;
  // 时长按源帧率换算成秒；缺失 ip/op 时兜底为 3 秒，避免除零或空动画。
  frameRate = Math.max(1, Number(data.fr) || 30);
  ip = Number.isFinite(data.ip) ? Number(data.ip) : 0;
  op = Number.isFinite(data.op) && Number(data.op) > ip ? Number(data.op) : ip + frameRate * 3;
  if (op <= ip) {
    op = ip + frameRate * 3;
  }
  duration = Math.max(0.001, (op - ip) / frameRate);
  // 默认按源帧率采样，并限制在 ABI 允许的 6–120 区间；换文档时圆角归零，其余滑块保持不变。
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
  // 新文档加载完自动开始播放（若用户之前手动暂停，这里也会重新开始）。
  setPlaying(true);
}

/** 载入内置示例：在 SAMPLES 中按 id 查找。 */
async function loadSample(id: string): Promise<void> {
  const sample = SAMPLES.find((item) => item.id === id);
  if (sample) {
    await loadDocument(sample.data, id);
  }
}

/** 载入 `public/animations/*.json` 下的外部示例（浏览器 fetch）。 */
async function loadDemo(name: string): Promise<void> {
  // 用 baseURI 拼相对路径，兼容部署在子目录下的情况。
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

/**
 * 刷新资源库列表；接口不可用（纯静态部署）时关闭资源库入口。
 * 首次失败后不再反复重试，避免每次启动都打一条 404。
 */
async function refreshLibrary(): Promise<void> {
  try {
    libraryFiles.value = await listLibrary();
    libraryAvailable.value = true;
  } catch {
    libraryAvailable.value = false;
  }
}

/**
 * 批量导入本地文件：逐个解析，成功的自动写入 data/ 资源库，最后载入第一个。
 * 拖入多个 JSON 即可一次完成“入库 + 预览”，便于大批量测试。
 */
async function loadFiles(input: File[]): Promise<void> {
  if (input.length === 0) {
    return;
  }
  const accepted: Array<{ name: string; data: LottieData }> = [];
  let rejected = 0;
  let saved = 0;
  let saveFailed = 0;
  libraryBusy.value = libraryAvailable.value;
  try {
    for (const [index, file] of input.entries()) {
      if (input.length > 1) {
        showToast(`正在处理 ${index + 1}/${input.length}：${file.name}`);
      }
      let text: string;
      let data: LottieData;
      try {
        text = await file.text();
        data = JSON.parse(text) as LottieData;
      } catch {
        rejected += 1;
        continue;
      }
      accepted.push({ name: file.name, data });
      if (!libraryAvailable.value) {
        continue;
      }
      try {
        await saveLibraryFile(file.name, text);
        saved += 1;
      } catch {
        // 入库失败不影响本地预览，结束后在提示里说明。
        saveFailed += 1;
      }
    }
  } finally {
    libraryBusy.value = false;
    if (saved > 0) {
      await refreshLibrary();
    }
  }

  if (accepted.length === 0) {
    showToast(rejected > 0 ? `导入失败：${rejected} 个文件不是合法 JSON` : '没有可用的文件');
    return;
  }

  await loadDocument(accepted[0].data, `lib:${accepted[0].name}`);

  const parts: string[] = [];
  if (saved > 0) {
    parts.push(`已入库 ${saved} 个文件到 data/`);
  }
  if (rejected > 0) {
    parts.push(`${rejected} 个解析失败`);
  }
  if (saveFailed > 0) {
    parts.push(`${saveFailed} 个入库失败`);
  }
  if (parts.length > 0) {
    showToast(parts.join(' · '));
  }
}

/** 从资源库载入一份文档；来源 id 用 `lib:` 前缀，供顶栏高亮与面板选中。 */
async function loadLibraryEntry(name: string): Promise<void> {
  if (!converter) {
    return;
  }
  try {
    const data = await readLibraryFile(name);
    await loadDocument(data, `lib:${name}`);
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error));
  }
}

/** 从 data/ 删除一份资源；当前正在预览的文件仍留在内存中，不受影响。 */
async function removeLibraryEntry(name: string): Promise<void> {
  try {
    await deleteLibraryFile(name);
    delete libraryStatus[name];
    await refreshLibrary();
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error));
  }
}

/**
 * 批量转换校验：按当前参数逐个转换资源库文件，把网格数 / 三角形数 / 耗时
 * 写进列表。转换是同步的，每处理完一个让出主线程，避免大批量时页面卡死。
 */
async function checkLibrary(): Promise<void> {
  if (!converter || libraryFiles.value.length === 0) {
    return;
  }
  libraryBusy.value = true;
  for (const key of Object.keys(libraryStatus)) {
    delete libraryStatus[key];
  }
  const options = convertOptions();
  let failed = 0;
  for (const file of libraryFiles.value) {
    try {
      const data = await readLibraryFile(file.name);
      const started = performance.now();
      const converted = converter.convert(data, options);
      const elapsed = performance.now() - started;
      libraryStatus[file.name] =
        `${converted.meshCount} 网格 · ${(converted.totalIndices / 3).toLocaleString()} 三角形 · ${elapsed.toFixed(1)} ms`;
    } catch (error) {
      failed += 1;
      libraryStatus[file.name] = `失败：${error instanceof Error ? error.message : String(error)}`;
    }
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  }
  libraryBusy.value = false;
  const total = libraryFiles.value.length;
  showToast(
    failed > 0
      ? `校验完成：${total - failed}/${total} 通过，${failed} 个失败`
      : `校验完成：${total} 个文件全部通过`,
  );
}

/** 顶部数据源切换：`demo:` 走外部 JSON，`library` 打开资源库，其余为内置示例 id。 */
function onSelectSource(id: string): void {
  if (id.startsWith('demo:')) {
    void loadDemo(id.slice(5));
  } else if (id === 'library') {
    libraryOpen.value = true;
    void refreshLibrary();
  } else {
    void loadSample(id);
  }
}

/** 触发隐藏的 file input，由其在 change 事件里真正读取文件。 */
function pickFile(): void {
  fileInput.value?.click();
}

/** 文件选择回调（支持多选）；清空 value 以便下次选择同名文件时仍能触发 change。 */
function onFileChange(event: Event): void {
  const input = event.target as HTMLInputElement;
  const files = Array.from(input.files ?? []);
  if (files.length > 0) {
    void loadFiles(files);
  }
  input.value = '';
}

/** 空格播放/暂停；仅当焦点不在输入控件（target 为 body）时响应，避免抢输入框的按键。 */
function onKeyDown(event: KeyboardEvent): void {
  if (event.code === 'Space' && event.target === document.body) {
    event.preventDefault();
    togglePlay();
  }
}

/**
 * 判断这次拖拽是否携带外部文件。
 * 节点列表的层级拖拽同样会触发 window 的 dragenter，若不加区分会误显示
 * 全屏“松开以载入 Lottie JSON”遮罩，把整个页面盖住。
 */
function isFileDrag(event: DragEvent): boolean {
  const types = event.dataTransfer?.types;
  return types != null && Array.from(types).includes('Files');
}

/**
 * 拖入计数：dragenter/dragleave 会随子元素冒泡反复触发，
 * 用 dragDepth 计数可以保证只有真正离开窗口时才隐藏遮罩。
 * 仅外部文件拖入时才显示遮罩，内部节点排序不触发。
 */
function onDragEnter(event: DragEvent): void {
  if (!isFileDrag(event)) {
    return;
  }
  event.preventDefault();
  dragDepth += 1;
  dropVisible.value = true;
}

/** 必须阻止默认行为，否则浏览器会直接打开被拖入的 JSON 文件。 */
function onDragOver(event: DragEvent): void {
  if (isFileDrag(event)) {
    event.preventDefault();
  }
}

function onDragLeave(): void {
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) {
    dropVisible.value = false;
  }
}

/** 释放文件：批量交给 loadFiles（全部入库，并预览第一个），并重置遮罩状态。 */
function onDrop(event: DragEvent): void {
  event.preventDefault();
  dragDepth = 0;
  dropVisible.value = false;
  const files = Array.from(event.dataTransfer?.files ?? []);
  if (files.length > 0) {
    void loadFiles(files);
  }
}

/** 帧率 HUD：每 500ms 统计一个窗口，顺带把平均帧率交给 Stage 做分辨率自适应。 */
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

/**
 * 全局播放时钟（rAF 驱动）。
 * `time` 以秒为单位，是本页面唯一的时间源；每帧先推进时间再 syncScene，把时间同步给
 * lottie-web 原画与 three.js 舞台，最后调 Stage 渲染。
 */
function tick(now: number): void {
  // 单帧步长上限 0.1s：标签页切回或掉帧时避免动画直接跳一大段。
  const delta = lastTick === 0 ? 0 : Math.min(0.1, (now - lastTick) / 1000);
  lastTick = now;

  if (playing.value && duration > 0) {
    time += delta * speed.value;
    // 播放到头后取模循环；用 % 而不是清零，可保住高倍速下越界的余量。
    if (time >= duration) {
      time %= duration;
    }
    syncScene();
  }
  stage?.render();
  updateHud(now);
  rafId = requestAnimationFrame(tick);
}

/** 启动流程：先加载 wasm 转换器，成功后探测资源库并载入默认示例。 */
async function boot(): Promise<void> {
  try {
    converter = await loadConverter();
  } catch (error) {
    showToast(`Rust 转换器加载失败：${error instanceof Error ? error.message : String(error)}`);
    return;
  }
  // 资源库探测不阻塞首屏：接口存在时顶栏会多出「资源库」入口。
  void refreshLibrary();
  await loadSample('orbit');
}

// 层深/厚度只改矩阵增量，无需重转换，直接即时生效。
watch(depth, (value) => stage?.setDepthScale(value));
watch(thickness, (value) => stage?.setThicknessScale(value));
// 自动环绕与视角锁定互斥：必须先解锁，否则 Stage 的 setAutoRotate 会因锁定而忽略请求。
watch(autoRotate, (value) => {
  if (value && axisLock.value) {
    axisLock.value = false;
    // 先解除锁定，再开启自动环绕。
    stage?.setRotationLock(false);
  }
  stage?.setAutoRotate(value);
});
// 锁定视角时自动关闭环绕（反向约束，避免两个开关同时为真）。
watch(axisLock, (value) => {
  if (value) {
    autoRotate.value = false;
  }
  stage?.setRotationLock(value);
});
// 这三个参数会改变几何/采样，需要重新跑 Rust 转换（经过 90ms 防抖）。
watch(edgeRatio, scheduleReconvert);
watch(sampleFps, scheduleReconvert);
watch(curveSpacing, scheduleReconvert);
// 柔光总开关：一次性把标记铺到所有节点，保证 UI 圆点与 Stage 判定一致。
// 之后仍可单独覆盖某个节点，即使总开关保持关闭。
watch(softGlow, (value) => {
  for (const node of nodes.value) {
    nodeGlow[node.id] = value;
  }
  stage?.setAllNodeGlow(value);
  stage?.setSoftGlow(value);
});
// 柔光强度 / 半径是渲染期参数，无需重新转换，暂停时也会立即重绘。
watch(glowIntensity, (value) => stage?.setGlowIntensity(value));
watch(glowRadius, (value) => stage?.setGlowRadius(value));

// 挂载：创建舞台并把各滑块初值灌进去，然后启动时钟与全局事件监听。
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

// 卸载：停掉时钟、摘除事件监听、清理定时器并释放 three.js / lottie 资源。
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
      :sources="sources"
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
            v-model:curve-spacing="curveSpacing"
            v-model:auto-rotate="autoRotate"
            v-model:axis-lock="axisLock"
            v-model:soft-glow="softGlow"
            v-model:glow-intensity="glowIntensity"
            v-model:glow-radius="glowRadius"
            :nodes="nodes"
            :selected="selectedIds"
            :glow="nodeGlow"
            :visible="nodeVisible"
            @select-node="onNodeSelect"
            @toggle-node-glow="toggleNodeGlow"
            @toggle-node-visibility="toggleNodeVisibility"
            @reorder-node="onNodeReorder"
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
      <span>支持 Bodymovin 导出的 .json · 可多选，自动存入 data/</span>
    </div>
  </div>

  <LibraryPanel
    v-if="libraryOpen"
    :files="libraryFiles"
    :active-name="activeLibraryName"
    :busy="libraryBusy"
    :statuses="libraryStatus"
    @close="libraryOpen = false"
    @load="loadLibraryEntry"
    @remove="removeLibraryEntry"
    @upload="pickFile"
    @check="checkLibrary"
  />

  <div class="toast" :class="{ 'is-visible': toastVisible }" role="status">
    {{ toastText }}
  </div>

  <input
    ref="fileInput"
    type="file"
    accept=".json,application/json"
    multiple
    hidden
    @change="onFileChange"
  />
</template>
