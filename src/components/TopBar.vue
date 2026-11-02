<script setup lang="ts">
interface SourceChip {
  id: string;
  label: string;
  accent?: boolean;
}

const props = defineProps<{
  sources: SourceChip[];
  activeSource: string;
  conversion: string;
  frames: string;
  geometry: string;
  size: string;
}>();

const emit = defineEmits<{
  (event: 'select', id: string): void;
  (event: 'pick'): void;
}>();

function onChip(chip: SourceChip): void {
  if (chip.id === 'pick') {
    emit('pick');
    return;
  }
  emit('select', chip.id);
}
</script>

<template>
  <header class="topbar">
    <div class="brand">
      <span class="brand-mark">L<span class="brand-arrow">→</span>3D</span>
      <div class="brand-copy">
        <h1>Lottie 2 3D</h1>
        <p>Rust 把矢量关键帧烘成可环绕的立体网格</p>
      </div>
    </div>

    <div class="sources" role="group" aria-label="动画来源">
      <button
        v-for="chip in props.sources"
        :key="chip.id"
        class="chip"
        :class="{ 'is-active': chip.id === props.activeSource, 'chip-accent': chip.accent }"
        type="button"
        @click="onChip(chip)"
      >
        {{ chip.label }}
      </button>
    </div>

    <div class="stats" aria-label="转换统计">
      <span class="stat">{{ props.conversion }}</span>
      <span class="stat">{{ props.frames }}</span>
      <span class="stat">{{ props.geometry }}</span>
      <span class="stat stat-strong">{{ props.size }}</span>
    </div>
  </header>
</template>
