<script setup lang="ts">
// 资源库面板：列出项目 data/ 里的 Lottie，支持载入、删除、上传与批量转换校验。
// 所有数据由 App.vue 传入，点击只上抛事件，面板本身不直接发请求。
import type { LibraryFile } from '../library';

const props = defineProps<{
  files: LibraryFile[];
  /** 当前正在预览的资源库文件名；为空表示当前文档不来自资源库。 */
  activeName: string;
  /** 上传或批量校验进行中，用于禁用按钮。 */
  busy: boolean;
  /** 批量校验结果，key 为文件名；以“失败”开头的字符串按错误样式显示。 */
  statuses: Record<string, string>;
}>();

const emit = defineEmits<{
  (event: 'close'): void;
  (event: 'load', name: string): void;
  (event: 'remove', name: string): void;
  (event: 'upload'): void;
  (event: 'check'): void;
}>();

/** 文件大小格式化成 KB / MB。 */
function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** 修改时间只显示“月-日 时:分”，资源库用于本地调试，年份没有意义。 */
function formatTime(value: number): string {
  const date = new Date(value);
  const pad = (part: number) => String(part).padStart(2, '0');
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
</script>

<template>
  <div class="library-overlay" @click.self="emit('close')">
    <section class="library-panel" role="dialog" aria-label="Lottie 资源库">
      <header class="library-head">
        <div>
          <h2>资源库</h2>
          <p>
            上传的 Lottie 保存在项目 <code>data/</code> 目录，共 {{ props.files.length }} 个文件
          </p>
        </div>
        <button class="ghost" type="button" @click="emit('close')">关闭</button>
      </header>

      <div class="library-actions">
        <button
          class="chip chip-accent"
          type="button"
          :disabled="props.busy"
          @click="emit('upload')"
        >
          {{ props.busy ? '处理中…' : '上传 Lottie' }}
        </button>
        <button
          class="ghost"
          type="button"
          :disabled="props.busy || props.files.length === 0"
          @click="emit('check')"
        >
          批量转换校验
        </button>
        <span class="library-hint">点击文件名载入；拖入多个 JSON 会自动入库</span>
      </div>

      <ul class="library-list">
        <li
          v-for="file in props.files"
          :key="file.name"
          class="library-row"
          :class="{ 'is-active': file.name === props.activeName }"
        >
          <button class="library-name" type="button" @click="emit('load', file.name)">
            <span class="library-title">{{ file.name }}</span>
            <span class="library-meta">{{ formatSize(file.size) }} · {{ formatTime(file.modified) }}</span>
          </button>
          <span
            v-if="props.statuses[file.name]"
            class="library-status"
            :class="{ 'is-error': props.statuses[file.name].startsWith('失败') }"
          >
            {{ props.statuses[file.name] }}
          </span>
          <button
            class="library-remove"
            type="button"
            title="从 data/ 删除"
            @click="emit('remove', file.name)"
          >
            ×
          </button>
        </li>
        <li v-if="props.files.length === 0" class="library-empty">
          资源库还是空的——拖入或点击「上传 Lottie」即可写入 data/。
        </li>
      </ul>
    </section>
  </div>
</template>
