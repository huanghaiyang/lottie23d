<script setup lang="ts">
// 底部播放条：播放/暂停、进度拖动、倍速、重置视角。状态全部由 App.vue 注入。
/** playing 播放状态；progress 为 0–1000 的滑块进度；frameLabel 帧文案；fps 实测帧率。 */
const props = defineProps<{
  playing: boolean;
  progress: number;
  frameLabel: string;
  fps: number;
}>();

// 拖动进度条时先发 drag-start（App 暂停播放），松开发 drag-end 再恢复。
const emit = defineEmits<{
  (event: 'toggle-play'): void;
  (event: 'seek', value: number): void;
  (event: 'drag-start'): void;
  (event: 'drag-end'): void;
  (event: 'reset-view'): void;
}>();

/** 播放倍速（0.25–2），双向绑定回 App 的播放时钟。 */
const speed = defineModel<number>('speed', { required: true });

/** 舞台选取开关：开启后可直接在 3D 画布上点击模型选中节点。 */
const pickMode = defineModel<boolean>('pick-mode', { required: true });

/** 把 range 的字符串值转成数字后上抛；进度到时间的换算由 App 负责。 */
function onSeek(event: Event): void {
  emit('seek', Number((event.target as HTMLInputElement).value));
}
</script>

<template>
  <footer class="transport">
    <button
      class="play"
      type="button"
      :data-playing="String(props.playing)"
      :aria-pressed="props.playing"
      @click="emit('toggle-play')"
    >
      <span>{{ props.playing ? '暂停' : '播放' }}</span>
    </button>

    <div class="timeline">
      <input
        type="range"
        min="0"
        max="1000"
        step="1"
        :value="props.progress"
        aria-label="动画进度"
        @input="onSeek"
        @pointerdown="emit('drag-start')"
        @pointerup="emit('drag-end')"
        @pointercancel="emit('drag-end')"
      />
      <span class="frame-readout">{{ props.frameLabel }}</span>
    </div>

    <div class="fields">
      <label class="field">
        <span>速度</span>
        <input v-model.number="speed" type="range" min="0.25" max="2" step="0.05" />
        <output>{{ speed.toFixed(2) }}×</output>
      </label>
    </div>

    <div class="view-actions">
      <button
        class="ghost pick-toggle"
        :class="{ 'is-active': pickMode }"
        type="button"
        :aria-pressed="pickMode"
        title="开启后可直接在 3D 画布上点击模型选取节点"
        @click="pickMode = !pickMode"
      >
        舞台选取
      </button>
      <button class="ghost" type="button" @click="emit('reset-view')">重置视角</button>
      <span class="fps">{{ props.fps > 0 ? `${props.fps} FPS` : '-- FPS' }}</span>
    </div>
  </footer>
</template>
