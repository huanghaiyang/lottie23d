<script setup lang="ts">
// 资源库面板：文件列表（分页）、单文件预览、多选批量改名、上传与批量转换校验。
// 列表与校验结果由 App.vue 传入；预览所需的文件内容由本组件直接读接口，
// 这样切换选中项时不必让 App 参与，也不会影响正在播放的 3D 场景。
import lottie, { type AnimationItem } from 'lottie-web';
import { computed, onBeforeUnmount, reactive, ref, watch } from 'vue';
import { readLibraryFile, type LibraryFile } from '../library';

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
  (event: 'rename', items: Array<{ from: string; to: string }>): void;
}>();

/* ------------------------------------------------------------------ 分页 */

/** 每页条数；批量测试时默认 24，翻页成本低又能一屏看完一批。 */
const pageSize = ref(24);
const page = ref(1);

const totalPages = computed(() => Math.max(1, Math.ceil(props.files.length / pageSize.value)));
/** 当前页的文件切片。 */
const pageFiles = computed(() => {
  const start = (page.value - 1) * pageSize.value;
  return props.files.slice(start, start + pageSize.value);
});
/** 当前页在总数中的区间文案，例如 `25–48`。 */
const rangeLabel = computed(() => {
  if (props.files.length === 0) {
    return '0';
  }
  const start = (page.value - 1) * pageSize.value + 1;
  const end = Math.min(props.files.length, page.value * pageSize.value);
  return `${start}–${end}`;
});

// 文件数量变化（上传 / 删除 / 改名）后把页码收进合法范围。
watch(
  () => props.files.length,
  () => {
    if (page.value > totalPages.value) {
      page.value = totalPages.value;
    }
  },
);
// 换每页条数时回到第一页，避免停在越界页。
watch(pageSize, () => {
  page.value = 1;
});

/* -------------------------------------------------------------- 多选与改名 */

/** 勾选集合；Vue 3 的 reactive Set 会正常参与依赖收集。 */
const selection = reactive(new Set<string>());
const selectedCount = computed(() => selection.size);
const allSelected = computed(
  () => pageFiles.value.length > 0 && pageFiles.value.every((file) => selection.has(file.name)),
);

function toggleSelect(name: string): void {
  if (selection.has(name)) {
    selection.delete(name);
  } else {
    selection.add(name);
  }
}

/** 全选 / 反选只作用于当前页，避免误选整库几百个文件。 */
function toggleSelectAll(): void {
  const target = pageFiles.value;
  if (allSelected.value) {
    target.forEach((file) => selection.delete(file.name));
  } else {
    target.forEach((file) => selection.add(file.name));
  }
}

function clearSelection(): void {
  selection.clear();
}

/** 改名方式：加前缀 / 加后缀 / 数字后缀（后缀 + 递增序号）/ 查找替换 / 序号命名。 */
type RenameMode = 'prefix' | 'suffix' | 'number' | 'replace' | 'sequence';

const renameOpen = ref(false);
const renameMode = ref<RenameMode>('prefix');
/** 前缀与序号模式下复用同一个输入，减少表单项。 */
const renamePrefix = ref('shot_');
/** 后缀与数字后缀模式共用；默认下划线，便于拼出 `名字_01`。 */
const renameSuffix = ref('_');
const renameFind = ref('');
const renameReplace = ref('');
const renameStart = ref(1);
const renamePad = ref(2);
const renameError = ref('');

/** 选中文件按当前列表顺序排列，保证序号命名连续且可预期。 */
const selectedFiles = computed(() => props.files.filter((file) => selection.has(file.name)));

/** 按当前规则生成改名计划；名称没变化的条目会被跳过。 */
const renamePlan = computed<Array<{ from: string; to: string }>>(() => {
  const plan: Array<{ from: string; to: string }> = [];
  selectedFiles.value.forEach((file, index) => {
    const base = file.name.replace(/\.json$/i, '');
    let next = base;
    if (renameMode.value === 'prefix') {
      next = `${renamePrefix.value}${base}`;
    } else if (renameMode.value === 'suffix') {
      next = `${base}${renameSuffix.value}`;
    } else if (renameMode.value === 'number') {
      // 数字后缀：保留原名，再追加后缀与递增序号，例如 `batch-01_01`。
      const serial = String(renameStart.value + index).padStart(Math.max(1, renamePad.value), '0');
      next = `${base}${renameSuffix.value}${serial}`;
    } else if (renameMode.value === 'replace') {
      // 查找串为空时不做任何事，避免出现无意义的替换。
      if (renameFind.value === '') {
        return;
      }
      next = base.split(renameFind.value).join(renameReplace.value);
    } else {
      const serial = String(renameStart.value + index).padStart(Math.max(1, renamePad.value), '0');
      next = `${renamePrefix.value}${serial}`;
    }
    const to = `${next}.json`;
    if (to !== file.name) {
      plan.push({ from: file.name, to });
    }
  });
  return plan;
});

/** 表单右侧的一行示例，让用户提交前能确认规则是否符合预期。 */
const renameHint = computed(() => {
  const plan = renamePlan.value;
  if (selection.size === 0) {
    return '先勾选需要改名的文件';
  }
  if (plan.length === 0) {
    return '新名称与原名相同';
  }
  const first = plan[0];
  return `${first.from} → ${first.to}${plan.length > 1 ? ` 等 ${plan.length} 项` : ''}`;
});

/** 校验计划后上抛；服务端会再校验一次文件名与冲突。 */
function applyRename(): void {
  const plan = renamePlan.value;
  if (plan.length === 0) {
    renameError.value = '没有需要改名的文件';
    return;
  }
  const targets = plan.map((item) => item.to);
  if (new Set(targets).size !== targets.length) {
    renameError.value = '新文件名有重复，请调整规则';
    return;
  }
  renameError.value = '';
  emit('rename', plan);
}

/* ------------------------------------------------------------------ 预览 */

const previewName = ref('');
const previewHost = ref<HTMLDivElement | null>(null);
const previewState = ref<'idle' | 'loading' | 'ready' | 'error'>('idle');
const previewError = ref('');
const previewMeta = ref('');
let previewAnimation: AnimationItem | null = null;

/** 释放预览实例；面板关闭、切换文件、文件被改名/删除时都要调用。 */
function disposePreview(): void {
  previewAnimation?.destroy();
  previewAnimation = null;
  previewHost.value?.replaceChildren();
}

/** 清空预览区，回到“未选择”状态。 */
function resetPreview(): void {
  disposePreview();
  previewName.value = '';
  previewMeta.value = '';
  previewState.value = 'idle';
  previewError.value = '';
}

/** 读取并播放选中文档；只渲染原画，不做 Rust 转换，切换很快。 */
async function selectPreview(name: string): Promise<void> {
  disposePreview();
  previewName.value = name;
  previewMeta.value = '';
  previewError.value = '';
  previewState.value = 'loading';
  try {
    const data = await readLibraryFile(name);
    // 请求期间用户可能已切到别的文件或关闭面板。
    if (previewName.value !== name || previewHost.value == null) {
      return;
    }
    const layers = Array.isArray(data.layers) ? data.layers.length : 0;
    const fps = Math.max(1, Number(data.fr) || 30);
    const frames = Math.max(0, (Number(data.op) || 0) - (Number(data.ip) || 0));
    previewMeta.value =
      `${Number(data.w) || 0} × ${Number(data.h) || 0} · ${fps.toFixed(0)} fps · ` +
      `${frames.toFixed(0)} 帧 · ${layers} 图层`;
    previewAnimation = lottie.loadAnimation({
      container: previewHost.value,
      renderer: 'canvas',
      loop: true,
      autoplay: true,
      animationData: data,
      rendererSettings: {
        clearCanvas: true,
        preserveAspectRatio: 'xMidYMid meet',
        dpr: Math.min(1.5, Math.max(1, window.devicePixelRatio || 1)),
      },
    });
    previewState.value = 'ready';
  } catch (error) {
    if (previewName.value !== name) {
      return;
    }
    previewError.value = error instanceof Error ? error.message : String(error);
    previewState.value = 'error';
  }
}

// 列表整体换新（上传 / 删除 / 改名成功后的刷新）时，清掉多选与改名表单；
// 预览中的文件若已不存在也一并关闭，避免展示过期内容。
watch(
  () => props.files,
  () => {
    selection.clear();
    renameOpen.value = false;
    renameError.value = '';
    if (previewName.value !== '' && !props.files.some((file) => file.name === previewName.value)) {
      resetPreview();
    }
  },
);

onBeforeUnmount(disposePreview);

/* ------------------------------------------------------------------ 格式化 */

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
        <label v-if="pageFiles.length > 0" class="library-select-all">
          <input type="checkbox" :checked="allSelected" @change="toggleSelectAll" />
          全选本页
        </label>
        <span class="library-hint">点击文件预览；勾选后可批量改名</span>
      </div>

      <div v-if="selectedCount > 0" class="library-batch">
        <span>已选 {{ selectedCount }} 项</span>
        <button class="chip" type="button" @click="renameOpen = !renameOpen">批量改名</button>
        <button class="ghost" type="button" @click="clearSelection">取消选择</button>
      </div>

      <form v-if="renameOpen && selectedCount > 0" class="library-rename" @submit.prevent="applyRename">
        <label class="library-field">
          方式
          <select v-model="renameMode">
            <option value="prefix">加前缀</option>
            <option value="suffix">加后缀</option>
            <option value="number">数字后缀</option>
            <option value="replace">查找替换</option>
            <option value="sequence">序号命名</option>
          </select>
        </label>
        <label v-if="renameMode === 'prefix' || renameMode === 'sequence'" class="library-field">
          {{ renameMode === 'sequence' ? '名称前缀' : '前缀' }}
          <input v-model="renamePrefix" type="text" placeholder="shot_" />
        </label>
        <label v-if="renameMode === 'suffix' || renameMode === 'number'" class="library-field">
          {{ renameMode === 'number' ? '连接符' : '后缀' }}
          <input v-model="renameSuffix" type="text" placeholder="_" />
        </label>
        <label v-if="renameMode === 'replace'" class="library-field">
          查找
          <input v-model="renameFind" type="text" placeholder="旧名片段" />
        </label>
        <label v-if="renameMode === 'replace'" class="library-field">
          替换为
          <input v-model="renameReplace" type="text" placeholder="新片段" />
        </label>
        <label v-if="renameMode === 'sequence' || renameMode === 'number'" class="library-field">
          起始
          <input v-model.number="renameStart" type="number" min="0" max="9999" />
        </label>
        <label v-if="renameMode === 'sequence' || renameMode === 'number'" class="library-field">
          位数
          <input v-model.number="renamePad" type="number" min="1" max="6" />
        </label>
        <span class="library-rename-hint">{{ renameHint }}</span>
        <button class="chip chip-accent" type="submit" :disabled="props.busy">应用改名</button>
      </form>
      <p v-if="renameError" class="library-error">{{ renameError }}</p>

      <div class="library-body">
        <ul class="library-list">
          <li
            v-for="file in pageFiles"
            :key="file.name"
            class="library-row"
            :class="{
              'is-active': file.name === props.activeName,
              'is-preview': file.name === previewName,
            }"
            @click="selectPreview(file.name)"
          >
            <input
              class="library-check"
              type="checkbox"
              :checked="selection.has(file.name)"
              :aria-label="`选择 ${file.name}`"
              @click.stop
              @change="toggleSelect(file.name)"
            />
            <span class="library-name">
              <span class="library-title">{{ file.name }}</span>
              <span class="library-meta">
                {{ formatSize(file.size) }} · {{ formatTime(file.modified) }}
              </span>
            </span>
            <span
              v-if="props.statuses[file.name]"
              class="library-status"
              :class="{ 'is-error': props.statuses[file.name].startsWith('失败') }"
            >
              {{ props.statuses[file.name] }}
            </span>
            <button class="ghost library-load" type="button" @click.stop="emit('load', file.name)">
              载入
            </button>
            <button
              class="library-remove"
              type="button"
              title="从 data/ 删除"
              @click.stop="emit('remove', file.name)"
            >
              ×
            </button>
          </li>
          <li v-if="props.files.length === 0" class="library-empty">
            资源库还是空的——拖入或点击「上传 Lottie」即可写入 data/。
          </li>
        </ul>

        <aside class="library-preview">
          <div class="library-preview-stage">
            <div ref="previewHost" class="library-preview-mount"></div>
            <span v-if="previewState === 'idle'" class="library-preview-hint">选择文件预览</span>
            <span v-else-if="previewState === 'loading'" class="library-preview-hint">加载中…</span>
            <span v-else-if="previewState === 'error'" class="library-preview-hint is-error">
              {{ previewError }}
            </span>
          </div>
          <div class="library-preview-info">
            <span class="library-title">{{ previewName || '未选择' }}</span>
            <span class="library-meta">{{ previewMeta || '—' }}</span>
            <button
              class="chip chip-accent"
              type="button"
              :disabled="previewState !== 'ready'"
              @click="previewName !== '' && emit('load', previewName)"
            >
              载入 3D
            </button>
          </div>
        </aside>
      </div>

      <footer class="library-foot">
        <span class="library-foot-info">
          第 {{ page }} / {{ totalPages }} 页 · 本页 {{ rangeLabel }} 个
        </span>
        <div class="library-page-actions">
          <select v-model.number="pageSize" class="library-page-size" aria-label="每页数量">
            <option :value="12">12 / 页</option>
            <option :value="24">24 / 页</option>
            <option :value="48">48 / 页</option>
          </select>
          <button class="ghost" type="button" :disabled="page <= 1" @click="page -= 1">上一页</button>
          <button class="ghost" type="button" :disabled="page >= totalPages" @click="page += 1">
            下一页
          </button>
        </div>
      </footer>
    </section>
  </div>
</template>
