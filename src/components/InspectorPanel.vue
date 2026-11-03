<script setup lang="ts">
// 右侧“转换参数 + 节点列表”面板。
// 数据流：滑块/开关通过 defineModel 双向绑定回 App.vue；节点行的选择、柔光、
// 显隐只向上抛事件，状态统一由 App.vue 维护，这里只负责渲染与修饰键映射。
import { ref } from 'vue';
import type { StageNode } from '../scene/stage';

// 以下 8 个参数通过 defineModel 与 App.vue 双向绑定（模板中使用 kebab-case 名称）。
// 厚度/层深是渲染期增量，边缘平滑/曲线采样/采样帧率会触发重转换。
const thickness = defineModel<number>('thickness', { required: true });
const depth = defineModel<number>('depth', { required: true });
const edgeRatio = defineModel<number>('edge-ratio', { required: true });
const sampleFps = defineModel<number>('sample-fps', { required: true });
/** 轮廓采样间距（像素/点）：越小曲线越平滑，但顶点与显存随之增加。 */
const curveSpacing = defineModel<number>('curve-spacing', { required: true });
const autoRotate = defineModel<boolean>('auto-rotate', { required: true });
const axisLock = defineModel<boolean>('axis-lock', { required: true });
const softGlow = defineModel<boolean>('soft-glow', { required: true });

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

/** 把鼠标修饰键映射成 App 约定的选择语义（macOS 的 meta 视作 ctrl）。 */
function onNodeClick(id: number, event: MouseEvent): void {
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
        <span class="inspector-label">边缘平滑</span>
        <input v-model.number="edgeRatio" type="range" min="0" max="100" step="1" />
        <output>{{ edgeRatio.toFixed(0) }}%</output>
      </label>

      <label class="inspector-row" title="采样间距越小，曲线越平滑，但顶点与显存越多">
        <span class="inspector-label">曲线采样</span>
        <input v-model.number="curveSpacing" type="range" min="1" max="8" step="0.5" />
        <output>{{ curveSpacing.toFixed(1) }} px</output>
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
            :class="{ 'is-selected': isSelected(node.id), 'is-hidden': !isVisible(node.id) }"
            @click="onNodeClick(node.id, $event)"
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
            <span class="node-type">{{ node.kind === 0 ? '填充' : '描边' }}</span>
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
        <p class="node-hint">点选/再点取消 · Shift 范围 · Alt 加选 · Alt+黄点：单节点柔光</p>
      </div>
    </div>
  </aside>
</template>
