<script setup lang="ts">
import { ref } from 'vue';
import type { StageNode } from '../scene/stage';

const thickness = defineModel<number>('thickness', { required: true });
const depth = defineModel<number>('depth', { required: true });
const edgeRatio = defineModel<number>('edge-ratio', { required: true });
const sampleFps = defineModel<number>('sample-fps', { required: true });
const autoRotate = defineModel<boolean>('auto-rotate', { required: true });
const axisLock = defineModel<boolean>('axis-lock', { required: true });
const softGlow = defineModel<boolean>('soft-glow', { required: true });

const props = defineProps<{
  nodes: StageNode[];
  selected: number[];
  glow: Record<number, boolean>;
  visible: Record<number, boolean>;
}>();

const emit = defineEmits<{
  (
    event: 'select-node',
    id: number,
    modifiers: { shift: boolean; alt: boolean; ctrl: boolean },
  ): void;
  (event: 'toggle-node-glow', id: number, solo: boolean): void;
  (event: 'toggle-node-visibility', id: number): void;
}>();

const open = ref(true);

function isSelected(id: number): boolean {
  return props.selected.includes(id);
}

function isGlow(id: number): boolean {
  return softGlow.value ? props.glow[id] !== false : props.glow[id] === true;
}

function isVisible(id: number): boolean {
  return props.visible[id] !== false;
}

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
