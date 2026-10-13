<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue';
import { useLogSession } from './composables/useLogSession';
import { decodeName, fileSize, streamName } from './display';
import { LIMITS, type LogRow } from './types';
import UiIcon from './components/UiIcon.vue';
import SourceList from './components/SourceList.vue';
import LogToolbar from './components/LogToolbar.vue';
import LogTable from './components/LogTable.vue';
import RecordInspector from './components/RecordInspector.vue';
import RecordingDrawer from './components/RecordingDrawer.vue';
const session = useLogSession();
const { state, selected, keyword, onlyMatches, paused, follow, connection, error, loading, uploadBusy, uploadName, saveBusy, historyBusy, pageBusy, historyMode, offset, total, nextOffset, retained, selectedRow, selectedIndex, contextRows, contextBusy, contextError, imported, stream, recordings, activeRecordings, filtered, missing, needsAttention, canOperate } = session;
const fileInput = ref<HTMLInputElement>();
const toolbar = ref<InstanceType<typeof LogToolbar>>();
const table = ref<InstanceType<typeof LogTable>>();
const sources = ref<HTMLElement>();
const sourceButton = ref<HTMLButtonElement>();
const detailButton = ref<HTMLButtonElement>();
const saveButton = ref<HTMLButtonElement>();
const saveMenu = ref<HTMLElement>();
const sourcesOpen = ref(false), detailsOpen = ref(false), recordingsOpen = ref(false), saveOpen = ref(false);
const narrow = ref(false), overlayDetails = ref(false), dragging = ref(false);
const help = ref<HTMLDialogElement>();
const toast = ref('');
const theme = ref(localStorage.getItem('log-print.theme') || 'light');
const matches = computed(() => keyword.value ? filtered.value.map((row, index) => row.text.includes(keyword.value) ? index : -1).filter(i => i >= 0) : []);
const matchIndex = computed(() => matches.value.indexOf(selectedIndex.value));
const importProblem = computed(() => imported.value?.state === 'failed' ? imported.value.error : '');
const displayState = computed(() => historyMode.value ? '历史快照' : paused.value ? '显示已暂停' : '实时查看');
const importActivity = computed(() => uploadBusy.value ? `正在上传 ${uploadName.value}` : imported.value?.state === 'importing' ? `正在导入 ${decodeName(imported.value.name)} · ${fileSize(imported.value.bytes)}` : '');
let toastTimer: ReturnType<typeof setTimeout> | undefined;
let dragDepth = 0;
let smallScreen: MediaQueryList;
let detailScreen: MediaQueryList;
function adapt() { narrow.value = smallScreen.matches; overlayDetails.value = detailScreen.matches; if (!narrow.value) sourcesOpen.value = false; }
function toggleTheme() { theme.value = theme.value === 'dark' ? 'light' : 'dark'; localStorage.setItem('log-print.theme', theme.value); }
function chooseFile() { if (!uploadBusy.value && state.value.import_enabled && canOperate.value) fileInput.value?.click(); }
function onFile(event: Event) { const input = event.target as HTMLInputElement; const file = input.files?.[0]; input.value = ''; if (file) { sourcesOpen.value = false; void session.upload(file); } }
function selectStream(id: string) { selected.value = id; sourcesOpen.value = false; detailsOpen.value = false; if (narrow.value) sourceButton.value?.focus(); }
async function openSources() { sourcesOpen.value = !sourcesOpen.value; if (sourcesOpen.value) { await nextTick(); sources.value?.querySelector<HTMLButtonElement>('button')?.focus(); } }
function closeSources() { sourcesOpen.value = false; sourceButton.value?.focus(); }
async function openDetails() { if (!selectedRow.value) return; detailsOpen.value = true; if (overlayDetails.value) { await nextTick(); document.querySelector<HTMLElement>('.record-inspector')?.focus(); } }
function closeDetails() { detailsOpen.value = false; detailButton.value?.focus(); }
function selectRow(row: LogRow) { session.inspect(row); }
async function navigate(direction: number) {
  if (!matches.value.length) return;
  const current = matchIndex.value;
  const next = current < 0 ? (direction > 0 ? 0 : matches.value.length - 1) : (current + direction + matches.value.length) % matches.value.length;
  const index = matches.value[next];
  follow.value = false; session.inspect(filtered.value[index]); await nextTick(); table.value?.reveal(index);
}
async function toggleSave() {
  saveOpen.value = !saveOpen.value;
  if (saveOpen.value) { await nextTick(); saveMenu.value?.querySelector<HTMLButtonElement>('button')?.focus(); }
}
async function startSave(mode: string) { saveOpen.value = false; if (await session.save(mode)) { notify('已开始保存原始日志'); recordingsOpen.value = true; } else saveButton.value?.focus(); }
function notify(text: string) { toast.value = text; clearTimeout(toastTimer); toastTimer = setTimeout(() => { toast.value = ''; }, 3000); }
async function copy(text: string) { try { await navigator.clipboard.writeText(text); notify('已复制到剪贴板'); } catch { error.value = '无法访问剪贴板，请选中文本后复制。'; } }
function drop(event: DragEvent) {
  dragDepth = 0; dragging.value = false;
  if (!canOperate.value || !state.value.import_enabled) { error.value = '当前服务未开放文件导入。'; return; }
  if ((event.dataTransfer?.files.length || 0) > 1) { error.value = '请一次导入一个文件。'; return; }
  const file = event.dataTransfer?.files[0]; if (file) void session.upload(file);
}
function dragEnter(event: DragEvent) { if (event.dataTransfer?.types.includes('Files')) { dragDepth++; dragging.value = true; } }
function dragLeave() { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) dragging.value = false; }
function outside(event: PointerEvent) { if (saveOpen.value && !saveMenu.value?.contains(event.target as Node) && !saveButton.value?.contains(event.target as Node)) saveOpen.value = false; }
function keydown(event: KeyboardEvent) {
  if (help.value?.open || recordingsOpen.value) return;
  const edit = (event.target as HTMLElement)?.closest('input,textarea,[contenteditable=true]');
  const modal = sourcesOpen.value && narrow.value ? sources.value : detailsOpen.value && overlayDetails.value ? document.querySelector<HTMLElement>('.record-inspector') : saveOpen.value ? saveMenu.value : null;
  if (event.key === 'Escape') {
    if (saveOpen.value) { saveOpen.value = false; saveButton.value?.focus(); }
    else if (sourcesOpen.value) closeSources();
    else if (detailsOpen.value) closeDetails();
    return;
  }
  if (modal && event.key === 'Tab') {
    const items = Array.from(modal.querySelectorAll<HTMLElement>('button:not(:disabled),a[href],input:not(:disabled),[tabindex="0"]'));
    const index = items.indexOf(document.activeElement as HTMLElement);
    if (!items.length) return;
    if (event.shiftKey && index <= 0) { event.preventDefault(); items.at(-1)?.focus(); }
    else if (!event.shiftKey && (index === items.length - 1 || index < 0)) { event.preventDefault(); items[0].focus(); }
    return;
  }
  if (modal) return;
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f') { event.preventDefault(); toolbar.value?.focus(); }
  else if (!edit && event.code === 'Space' && !(event.target as HTMLElement).closest('button,a,summary')) { event.preventDefault(); session.togglePause(); }
  else if (!edit && event.key === '?') { event.preventDefault(); help.value?.showModal(); }
}
onMounted(() => {
  smallScreen = matchMedia('(max-width: 899px)'); detailScreen = matchMedia('(max-width: 1199px)'); adapt();
  smallScreen.addEventListener('change', adapt); detailScreen.addEventListener('change', adapt);
  window.addEventListener('keydown', keydown); window.addEventListener('pointerdown', outside);
});
onBeforeUnmount(() => { smallScreen?.removeEventListener('change', adapt); detailScreen?.removeEventListener('change', adapt); window.removeEventListener('keydown', keydown); window.removeEventListener('pointerdown', outside); clearTimeout(toastTimer); });
</script>

<template>
  <div class="log-app" :data-theme="theme" @dragenter.prevent="dragEnter" @dragover.prevent @dragleave.prevent="dragLeave" @drop.prevent="drop">
    <header class="app-header"><div class="brand"><img class="brand-logo" src="/log-print-mark.svg" alt="log_print LP logo" width="24" height="24" /><span>log_print</span><span class="product-name">日志查看器</span></div><div class="header-actions"><span class="connection" role="status"><i :class="['status-dot', connection]" />{{ connection === 'online' ? '本地已连接' : connection === 'offline' ? '连接中断 · 重连中' : '连接中' }}</span><span class="header-separator" /><button class="icon-button" :aria-label="theme === 'dark' ? '切换浅色模式' : '切换深色模式'" :title="theme === 'dark' ? '浅色模式' : '深色模式'" @click="toggleTheme"><UiIcon :name="theme === 'dark' ? 'sun' : 'moon'" /></button><button class="icon-button" aria-label="快捷键与使用帮助" title="快捷键与帮助 · ?" @click="help?.showModal()"><UiIcon name="help" /></button></div></header>
    <div class="workspace">
      <button v-if="sourcesOpen && narrow" class="drawer-scrim" aria-label="关闭来源遮罩" tabindex="-1" @click="closeSources" />
      <aside id="sources" ref="sources" class="source-sidebar" :class="{ 'is-open': sourcesOpen }" :role="narrow ? 'dialog' : 'complementary'" :aria-modal="sourcesOpen && narrow || undefined" aria-label="来源"><SourceList :streams="state.streams" :imports="state.imports" :selected="selected" :upload-busy="uploadBusy" :import-enabled="Boolean(state.import_enabled)" :connected="canOperate" @select="selectStream" @upload="chooseFile" @close="closeSources" /></aside>
      <main class="log-main" aria-label="日志阅读区">
        <header class="session-toolbar"><div class="stream-title"><button ref="sourceButton" class="icon-button mobile-only" aria-label="打开来源" aria-controls="sources" :aria-expanded="sourcesOpen" @click="openSources"><UiIcon name="menu" /></button><UiIcon :name="imported ? 'file' : 'stream'" /><h1 :title="streamName(stream)">{{ streamName(stream) }}</h1><span v-if="imported?.state === 'complete'" class="file-badge" title="文件导入完成">文件</span></div><div class="session-actions"><button class="button recording-toggle" :class="{ 'is-recording': activeRecordings.length }" aria-label="保存记录" @click="recordingsOpen = true"><UiIcon name="history" /><span>保存记录</span><b v-if="recordings.length" class="count">{{ recordings.length }}</b></button><div class="save-control"><button ref="saveButton" class="button primary" :disabled="!selected || saveBusy || !canOperate" :aria-expanded="saveOpen" aria-controls="save-options" @click="toggleSave"><UiIcon name="save" /><span>保存日志</span><UiIcon name="down" :size="12" /></button><div v-if="saveOpen" id="save-options" ref="saveMenu" class="save-popover" role="group" aria-label="选择保存起点"><div class="popover-label">保存当前流的原始日志</div><button @click="startSave('now')"><UiIcon name="play" /><span><strong>从现在开始保存</strong><small>记录此刻之后到达的数据</small></span></button><button @click="startSave('earliest')"><UiIcon name="history" /><span><strong>从最早可用数据保存</strong><small>补入已有历史，继续记录新数据</small></span></button></div></div></div></header>
        <LogToolbar ref="toolbar" v-model:keyword="keyword" v-model:only-matches="onlyMatches" :has-stream="Boolean(selected)" :matches="matches.length" :match-index="matchIndex" :history="historyMode" @navigate="navigate" />
        <div class="view-toolbar"><div class="view-mode" role="group" aria-label="查看范围"><button :class="{ active: !historyMode }" :aria-pressed="!historyMode" :disabled="!selected" @click="session.live">实时</button><button :class="{ active: historyMode }" :aria-pressed="historyMode" :disabled="!selected || !canOperate" @click="session.history">历史</button></div><span class="view-divider" /><button class="text-button pause-control" :class="{ 'is-paused': paused }" :disabled="!selected || historyMode" :title="paused ? '继续显示 · Space' : '仅暂停显示，后台采集和保存继续 · Space'" @click="session.togglePause"><UiIcon :name="paused ? 'play' : 'pause'" :size="14" />{{ paused ? '继续显示' : '暂停显示' }}</button><label class="follow-control"><input v-model="follow" type="checkbox" :disabled="historyMode || paused || !selected" /><span>跟随最新</span></label><span class="view-spacer" /><button class="icon-button" :disabled="!selectedRow" aria-label="复制所选日志" title="复制所选日志" @click="selectedRow && copy(selectedRow.text)"><UiIcon name="copy" /></button><button ref="detailButton" class="icon-button" :class="{ active: detailsOpen }" :disabled="!selectedRow" :aria-expanded="detailsOpen" aria-label="日志详情" title="查看所选日志详情 · Enter" @click="detailsOpen ? closeDetails() : openDetails()"><UiIcon name="detail" /></button></div>
        <div v-if="error || importProblem" class="notice error-notice" role="alert"><UiIcon name="warning" /><span>{{ error || importProblem }}</span><button v-if="error" class="icon-button" aria-label="关闭错误" @click="error = ''"><UiIcon name="close" /></button></div>
        <div v-if="importActivity || historyBusy" class="notice activity-notice" role="status"><span class="loading-dot" /><span>{{ importActivity || '正在扫描可用历史…' }}</span><button v-if="historyBusy" class="text-button" @click="session.live">取消读取</button></div>
        <div v-if="missing || needsAttention" class="notice warning-notice"><UiIcon name="warning" /><span>{{ needsAttention || '可用范围存在缺口，未获取到的日志不会被视为完整历史。' }}</span></div>
        <div class="reading-area"><LogTable ref="table" v-model:follow="follow" :rows="filtered" :selected="selectedRow" :keyword="keyword" :paused="paused" :history="historyMode" :offset="offset" :total="total" :has-stream="Boolean(selected)" :loading="loading || historyBusy || pageBusy" :filtering="onlyMatches" :import-enabled="Boolean(state.import_enabled) && canOperate" @select="selectRow" @inspect="openDetails" @upload="chooseFile" @clear="keyword = ''; onlyMatches = false" /><button v-if="detailsOpen && overlayDetails" class="drawer-scrim inspector-scrim" aria-label="关闭详情遮罩" tabindex="-1" @click="closeDetails" /><RecordInspector v-if="detailsOpen && selectedRow" :row="selectedRow" :stream-name="streamName(state.streams.find(s => s.id === selectedRow?.stream))" :context-rows="contextRows" :busy="contextBusy" :error="contextError" :overlay="overlayDetails" @close="closeDetails" @context="session.context" @copy="copy" /></div>
        <footer class="log-footer"><span class="display-state"><i :class="['status-dot', { live: !historyMode && !paused && canOperate, paused }]" />{{ displayState }}</span><span v-if="historyMode" class="range-count">{{ total ? offset + 1 : 0 }}–{{ nextOffset }} / {{ total.toLocaleString() }} 行</span><span v-else class="range-count">{{ filtered.length.toLocaleString() }} 行<span class="footer-secondary"> · 缓存 {{ retained.toLocaleString() }} 行</span></span><span class="footer-spacer" /><div v-if="historyMode" class="pagination"><button class="icon-button" :disabled="!offset || historyBusy || pageBusy" aria-label="上一页" @click="session.page(-1)"><UiIcon name="left" :size="14" /></button><button class="icon-button" :disabled="nextOffset >= total || historyBusy || pageBusy" aria-label="下一页" @click="session.page(1)"><UiIcon name="right" :size="14" /></button></div><span v-else class="footer-secondary">窗口最多 {{ LIMITS.liveRows.toLocaleString() }} 行</span><span class="encoding">UTF-8</span></footer>
      </main>
    </div>
    <input ref="fileInput" class="file-input" type="file" aria-label="选择日志文件" @change="onFile" />
    <div v-if="dragging" class="drop-overlay"><UiIcon name="upload" :size="32" /><strong>松开以导入日志</strong><span>每次一个文件 · 最大 64 MiB</span></div>
    <div v-if="toast" class="toast" role="status"><UiIcon name="check" />{{ toast }}</div>
    <RecordingDrawer v-if="recordingsOpen" :recordings="recordings" :streams="state.streams" :busy="saveBusy" :feedback="toast" @close="recordingsOpen = false" @stop="session.stop" @copy="copy" />
    <dialog ref="help" class="help-dialog" aria-labelledby="help-title"><header class="panel-title"><h2 id="help-title">使用日志查看器</h2><button class="icon-button" aria-label="关闭帮助" @click="help?.close()"><UiIcon name="close" /></button></header><div class="help-body"><h3>常用快捷键</h3><dl class="shortcuts"><div><dt>查找日志</dt><dd><kbd>⌘ / Ctrl</kbd><kbd>F</kbd></dd></div><div><dt>上 / 下一处匹配</dt><dd><kbd>Shift Enter</kbd> / <kbd>Enter</kbd></dd></div><div><dt>暂停 / 继续显示</dt><dd><kbd>Space</kbd></dd></div><div><dt>选择日志行</dt><dd><kbd>↑</kbd><kbd>↓</kbd></dd></div><div><dt>所选日志详情</dt><dd><kbd>Enter</kbd></dd></div><div><dt>关闭浮层</dt><dd><kbd>Esc</kbd></dd></div></dl><h3>查看与保存</h3><p>实时查看最近 {{ LIMITS.liveRows.toLocaleString() }} 行；历史查询覆盖当前可读取的缓存与归档。暂停和筛选只影响显示，不影响后台采集和日志保存。</p><p>文件保留原始内容。保存记录中的“已保存”表示文件已经提交，可直接下载。</p></div></dialog>
  </div>
</template>
