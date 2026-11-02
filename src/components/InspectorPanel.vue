<script setup lang="ts">
import { ref } from 'vue';

const thickness = defineModel<number>('thickness', { required: true });
const depth = defineModel<number>('depth', { required: true });
const edgeRatio = defineModel<number>('edge-ratio', { required: true });
const sampleFps = defineModel<number>('sample-fps', { required: true });
const autoRotate = defineModel<boolean>('auto-rotate', { required: true });
const axisLock = defineModel<boolean>('axis-lock', { required: true });

const open = ref(true);
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
    </div>
  </aside>
</template>
