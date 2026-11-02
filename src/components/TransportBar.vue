<script setup lang="ts">
const props = defineProps<{
  playing: boolean;
  progress: number;
  frameLabel: string;
  fps: number;
}>();

const emit = defineEmits<{
  (event: 'toggle-play'): void;
  (event: 'seek', value: number): void;
  (event: 'drag-start'): void;
  (event: 'drag-end'): void;
  (event: 'reset-view'): void;
}>();

const speed = defineModel<number>('speed', { required: true });

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
      <button class="ghost" type="button" @click="emit('reset-view')">重置视角</button>
      <span class="fps">{{ props.fps > 0 ? `${props.fps} FPS` : '-- FPS' }}</span>
    </div>
  </footer>
</template>
