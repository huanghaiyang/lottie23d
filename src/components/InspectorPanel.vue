<script setup lang="ts">
// 右侧“转换参数 + 节点列表”面板。
// 数据流：滑块/开关通过 defineModel 双向绑定回 App.vue；节点行的选择、柔光、
// 显隐只向上抛事件，状态统一由 App.vue 维护，这里只负责渲染与修饰键映射。
import { ref } from 'vue';
import type { StageNode } from '../scene/stage';

// 以下 8 个参数通过 defineModel 与 App.vue 双向绑定（模板中使用 kebab-case 名称）。
// 厚度/层深/柔光强度/柔光半径是渲染期增量，采样帧率会触发重转换。
// 边缘平滑与曲线采样暂不在面板暴露（实现尚有问题，选项保留在 App 内使用默认值）。
const thickness = defineModel<number>('thickness', { required: true });
const depth = defineModel<number>('depth', { required: true });
const sampleFps = defineModel<number>('sample-fps', { required: true });
const autoRotate = defineModel<boolean>('auto-rotate', { required: true });
const axisLock = defineModel<boolean>('axis-lock', { required: true });
const softGlow = defineModel<boolean>('soft-glow', { required: true });
/** 柔光强度倍率：1 为默认，0 相当于关闭叠加。 */
const glowIntensity = defineModel<number>('glow-intensity', { required: true });
/** 柔光半径倍率：1 为默认，越大越柔和。 */
const glowRadius = defineModel<number>('glow-radius', { required: true });

/** App.vue 注入的只读状态：节点列表、选中集合，以及逐节点柔光/显隐标记。 */
const props = defineProps<{
  nodes: StageNode[];
  selected: number[];
  glow: Record<number, boolean>;
  visible: Record<number, boolean>;
}>();

/** 节点交互统一上抛，避免面板与 App 各存一份状态导致不同步。 */
const emit = defineEmits<{
  (
    event: 'select-node',
    id: number,
    modifiers: { shift: boolean; alt: boolean; ctrl: boolean },
  ): void;
  (event: 'toggle-node-glow', id: number, solo: boolean): void;
  (event: 'toggle-node-visibility', id: number): void;
  /** 悬停节点行时高亮舞台中的对应图层；id 为 null 表示清除。 */
  (event: 'hover-node', id: number | null): void;
  /** 拖拽调整节点层级：把 fromId 插入到 toId 之前或之后。 */
  (event: 'reorder-node', fromId: number, toId: number, place: 'before' | 'after'): void;
}>();

/** 面板折叠状态；纯 UI 本地状态，不影响场景。 */
const open = ref(true);

/** 该节点是否在当前选中集合内。 */
function isSelected(id: number): boolean {
  return props.selected.includes(id);
}

/**
 * 柔光圆点是否点亮，语义与 Stage / App 保持一致：
 * 总开关开时默认点亮（除非显式 false），总开关关时仅显式 true 的节点点亮。
 */
function isGlow(id: number): boolean {
  return softGlow.value ? props.glow[id] !== false : props.glow[id] === true;
}

/** 节点是否可见；没有显式关闭记录的节点默认可见。 */
function isVisible(id: number): boolean {
  return props.visible[id] !== false;
}

/** 网格类型标签：0 填充、1 描边、2 图片贴图。 */
function nodeTypeLabel(kind: number): string {
  if (kind === 0) return '填充';
  if (kind === 2) return '图片';
  return '描边';
}

// ---- 拖拽重排 ----
/** 正在拖拽的节点 id；null 表示没有拖拽。 */
const dragId = ref<number | null>(null);
/** 当前悬停的目标节点 id，以及插入到它的上方还是下方。 */
const dropId = ref<number | null>(null);
const dropAfter = ref(false);
/** 拖拽结束后浏览器可能补发一次 click，需要忽略它。 */
let suppressClick = false;

/** 开始拖拽：记录源节点并写入 text/plain，Firefox 才会真正启动拖拽。 */
function onDragStart(node: StageNode, event: DragEvent): void {
  dragId.value = node.id;
  dropId.value = null;
  dropAfter.value = false;
  suppressClick = true;
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move';
    event.dataTransfer.setData('text/plain', String(node.id));
  }
}

/** 悬停到某行：按指针位于行的上半/下半决定插入方向。 */
function onDragOver(node: StageNode, event: DragEvent): void {
  if (dragId.value == null || dragId.value === node.id) {
    return;
  }
  event.preventDefault();
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'move';
  }
  dropId.value = node.id;
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  dropAfter.value = event.clientY > rect.top + rect.height / 2;
}

/** 离开目标行时清除插入指示，避免残留高亮。 */
function onDragLeave(node: StageNode): void {
  if (dropId.value === node.id) {
    dropId.value = null;
  }
}

/** 放下：把源节点插入目标行的上方或下方。 */
function onDrop(node: StageNode, event: DragEvent): void {
  event.preventDefault();
  const fromId = dragId.value;
  const place: 'before' | 'after' = dropAfter.value ? 'after' : 'before';
  resetDrag();
  if (fromId == null || fromId === node.id) {
    return;
  }
  emit('reorder-node', fromId, node.id, place);
}

/** 拖拽结束（包括拖到列表外取消）时清理全部临时状态。 */
function onDragEnd(): void {
  resetDrag();
  // click 通常紧随 dragend 同步派发，延后一拍再恢复选择响应。
  window.setTimeout(() => {
    suppressClick = false;
  }, 0);
}

function resetDrag(): void {
  dragId.value = null;
  dropId.value = null;
  dropAfter.value = false;
}

/** 把鼠标修饰键映射成 App 约定的选择语义（macOS 的 meta 视作 ctrl）。 */
function onNodeClick(id: number, event: MouseEvent): void {
  // 刚结束一次拖拽时忽略补发的 click，避免误切换选中项。
  if (suppressClick) {
    return;
  }
  emit('select-node', id, {
    shift: event.shiftKey,
    alt: event.altKey,
    ctrl: event.ctrlKey || event.metaKey,
  });
}
</script>

<template>
  <aside class="inspector" :class="{ 'is-collapsed': !open }">
    <button class="inspector-head" type="button" @click="open = !open">
      <span>转换参数</span>
      <span class="inspector-caret">{{ open ? '收起' : '展开' }}</span>
    </button>

    <div v-show="open" class="inspector-body">
      <label class="inspector-row">
        <span class="inspector-label">厚度</span>
        <input v-model.number="thickness" type="range" min="0.25" max="2.5" step="0.05" />
        <output>{{ thickness.toFixed(2) }}×</output>
      </label>

      <label class="inspector-row">
        <span class="inspector-label">层深</span>
        <input v-model.number="depth" type="range" min="0" max="2.5" step="0.05" />
        <output>{{ depth.toFixed(2) }}</output>
      </label>

      <label class="inspector-row">
        <span class="inspector-label">采样帧率</span>
        <input v-model.number="sampleFps" type="range" min="6" max="120" step="1" />
        <output>{{ sampleFps.toFixed(0) }} fps</output>
      </label>

      <label class="toggle inspector-toggle">
        <input v-model="autoRotate" type="checkbox" />
        <span class="toggle-track"><span class="toggle-thumb"></span></span>
        <span>自动环绕</span>
      </label>

      <label class="toggle">
        <input v-model="axisLock" type="checkbox" />
        <span class="toggle-track"><span class="toggle-thumb"></span></span>
        <span>视角锁定</span>
      </label>

      <label class="toggle">
        <input v-model="softGlow" type="checkbox" />
        <span class="toggle-track"><span class="toggle-thumb"></span></span>
        <span>柔光</span>
      </label>

      <label class="inspector-row" title="柔光叠加的亮度，0 相当于关闭，1 为默认">
        <span class="inspector-label">柔光强度</span>
        <input v-model.number="glowIntensity" type="range" min="0" max="2" step="0.05" />
        <output>{{ glowIntensity.toFixed(2) }}×</output>
      </label>

      <label class="inspector-row" title="模糊半径越大越柔和，过高会因 5 抽样出现轻微分层">
        <span class="inspector-label">柔光半径</span>
        <input v-model.number="glowRadius" type="range" min="0.2" max="3" step="0.05" />
        <output>{{ glowRadius.toFixed(2) }}×</output>
      </label>

      <div class="node-section">
        <div class="node-head">
          <span>节点</span>
          <span class="node-count">{{ nodes.length }}</span>
        </div>
        <div class="node-list">
          <span v-if="nodes.length === 0" class="node-empty">暂无节点</span>
          <div
            v-for="node in nodes"
            :key="node.id"
            class="node-row"
            :class="{
              'is-selected': isSelected(node.id),
              'is-hidden': !isVisible(node.id),
              'is-dragging': dragId === node.id,
              'is-drop-before': dropId === node.id && !dropAfter,
              'is-drop-after': dropId === node.id && dropAfter,
            }"
            draggable="true"
            @dragstart="onDragStart(node, $event)"
            @dragover="onDragOver(node, $event)"
            @dragleave="onDragLeave(node)"
            @drop="onDrop(node, $event)"
            @dragend="onDragEnd"
            @click="onNodeClick(node.id, $event)"
            @mouseenter="emit('hover-node', node.id)"
            @mouseleave="emit('hover-node', null)"
          >
            <span
              class="node-glow"
              :class="{ 'is-on': isGlow(node.id) }"
              :title="
                isGlow(node.id)
                  ? '关闭该节点柔光（Alt+点击：仅此节点）'
                  : '开启该节点柔光（总柔光关闭时也可单独开启）'
              "
              @click.stop="emit('toggle-node-glow', node.id, $event.altKey)"
            ></span>
            <span class="node-name" :title="node.name">{{ node.name }}</span>
            <span class="node-type">{{ nodeTypeLabel(node.kind) }}</span>
            <button
              class="node-vis"
              :class="{ 'is-off': !isVisible(node.id) }"
              type="button"
              :title="isVisible(node.id) ? '隐藏节点' : '显示节点'"
              @click.stop="emit('toggle-node-visibility', node.id)"
            >
              <svg viewBox="0 0 16 16" aria-hidden="true">
                <path d="M1.2 8S3.9 3.9 8 3.9 14.8 8 14.8 8 12.1 12.1 8 12.1 1.2 8 1.2 8Z" />
                <circle cx="8" cy="8" r="1.9" />
                <path v-if="!isVisible(node.id)" d="M3 3 13 13" />
              </svg>
            </button>
          </div>
        </div>
        <p class="node-hint">
          拖拽调整层级（上=后，下=前） · 点选/再点取消 · Shift 范围 · Alt 加选 · Alt+黄点：单节点柔光
        </p>
      </div>
    </div>
  </aside>
</template>
