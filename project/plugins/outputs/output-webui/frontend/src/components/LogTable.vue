<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { LIMITS, type LogRow } from '../types';
import { clockTime, rowKey } from '../display';
import { highlight } from '../logText';
import UiIcon from './UiIcon.vue';
const props = defineProps<{ rows: LogRow[]; selected: LogRow | null; keyword: string; paused: boolean; follow: boolean; history: boolean; offset: number; total: number; hasStream: boolean; loading: boolean; filtering: boolean; importEnabled: boolean }>();
const emit = defineEmits<{ select: [row: LogRow]; 'update:follow': [value: boolean]; upload: []; clear: []; inspect: [] }>();
const viewport = ref<HTMLElement>();
const top = ref(0), height = ref(500);
const first = computed(() => Math.max(0, Math.floor(top.value / LIMITS.rowHeight) - LIMITS.overscan));
const visible = computed(() => props.rows.slice(first.value, first.value + Math.ceil(height.value / LIMITS.rowHeight) + LIMITS.overscan * 2));
const selectedIndex = computed(() => props.selected ? props.rows.findIndex(row => rowKey(row) === rowKey(props.selected!)) : -1);
const activeId = computed(() => selectedIndex.value >= first.value && selectedIndex.value < first.value + visible.value.length ? `log-row-${selectedIndex.value}` : undefined);
let resize: ResizeObserver | undefined;
let moving = false;
async function bottom() {
  await nextTick();
  if (!viewport.value) return;
  moving = true; viewport.value.scrollTop = viewport.value.scrollHeight;
  top.value = viewport.value.scrollTop;
  requestAnimationFrame(() => { moving = false; });
}
function scroll() {
  if (!viewport.value) return;
  top.value = viewport.value.scrollTop;
  if (!moving && !props.history) emit('update:follow', viewport.value.scrollHeight - viewport.value.scrollTop - viewport.value.clientHeight < LIMITS.rowHeight * 2);
}
function reveal(index: number) {
  const element = viewport.value;
  if (!element || index < 0) return;
  if (index * LIMITS.rowHeight < element.scrollTop) element.scrollTop = index * LIMITS.rowHeight;
  else if ((index + 1) * LIMITS.rowHeight > element.scrollTop + element.clientHeight) element.scrollTop = (index + 1) * LIMITS.rowHeight - element.clientHeight;
  top.value = element.scrollTop;
  void nextTick().then(() => {
    const row = element.querySelector<HTMLElement>(`#log-row-${index}`);
    const mark = row?.querySelector<HTMLElement>('mark');
    if (!mark) return;
    const bounds = element.getBoundingClientRect();
    const hit = mark.getBoundingClientRect();
    const fixed = (row?.querySelector<HTMLElement>('.line-position')?.offsetWidth || 0) + (row?.querySelector<HTMLElement>('time')?.offsetWidth || 0);
    if (hit.right > bounds.right - 12) element.scrollLeft += hit.right - bounds.right + 12;
    else if (hit.left < bounds.left + fixed) element.scrollLeft += hit.left - bounds.left - fixed - 12;
  });
}
function choose(index: number) {
  const row = props.rows[index];
  if (!row) return;
  emit('update:follow', false); emit('select', row); reveal(index);
}
function keyboard(event: KeyboardEvent) {
  if (event.key === 'Enter' && props.selected) { event.preventDefault(); emit('inspect'); return; }
  const delta = ({ ArrowDown: 1, ArrowUp: -1, PageDown: Math.floor(height.value / LIMITS.rowHeight), PageUp: -Math.floor(height.value / LIMITS.rowHeight) } as Record<string, number>)[event.key];
  let next = selectedIndex.value < 0 ? 0 : selectedIndex.value;
  if (delta) next = selectedIndex.value < 0 ? (delta > 0 ? 0 : props.rows.length - 1) : next + delta;
  else if (event.key === 'Home') next = 0;
  else if (event.key === 'End') next = props.rows.length - 1;
  else return;
  event.preventDefault(); choose(Math.max(0, Math.min(props.rows.length - 1, next)));
}
watch(() => props.rows, async () => {
  if (props.follow && !props.paused && !props.history) await bottom();
  else { await nextTick(); if (viewport.value) top.value = viewport.value.scrollTop; }
});
watch(() => props.follow, value => { if (value && !props.history) void bottom(); });
watch(() => [props.history, props.offset, props.filtering, props.keyword], () => { if (viewport.value) { viewport.value.scrollTop = 0; top.value = 0; } });
onMounted(() => { resize = new ResizeObserver(entries => { height.value = entries[0]?.contentRect.height || 500; if (props.keyword && selectedIndex.value >= 0) reveal(selectedIndex.value); }); if (viewport.value) resize.observe(viewport.value); });
onBeforeUnmount(() => resize?.disconnect());
defineExpose({ reveal, bottom, focus: () => viewport.value?.focus() });
</script>
<template>
  <div class="log-table" :style="{ '--log-row-height': `${LIMITS.rowHeight}px` }">
    <div class="column-head" aria-hidden="true"><span>位置</span><span>采集时间</span><span>日志内容</span><span class="column-note">{{ history ? '固定历史快照' : paused ? '显示已暂停' : '按接收顺序' }}</span></div>
    <div ref="viewport" class="log-viewport" role="grid" aria-label="日志列表" aria-colcount="3" :aria-rowcount="history ? total : rows.length" :aria-activedescendant="activeId" :aria-busy="loading" tabindex="0" @scroll="scroll" @keydown="keyboard">
      <div v-if="!rows.length" class="log-empty">
        <UiIcon :name="loading ? 'history' : hasStream ? 'search' : 'file'" :size="32" />
        <h2>{{ loading ? '正在读取日志' : !hasStream ? '从一份日志开始' : filtering && keyword ? '没有匹配的日志' : '等待日志到达' }}</h2>
        <p>{{ loading ? '读取期间可以继续操作其他来源。' : !hasStream ? '拖入文件，或选择正在采集的来源。' : filtering && keyword ? '调整关键词，或清除筛选查看全部内容。' : '来源写入新内容后，会自动出现在这里。' }}</p>
        <button v-if="!hasStream" class="button primary" :disabled="!importEnabled" @click="emit('upload')"><UiIcon name="upload" />导入日志文件</button>
        <button v-else-if="filtering && keyword" class="button" @click="emit('clear')">清除筛选</button>
        <small v-if="!hasStream">纯文本日志 · UTF-8 · 最大 64 MiB</small>
      </div>
      <div v-else class="log-space" :style="{ height: `${rows.length * LIMITS.rowHeight}px` }">
        <div class="log-window" :style="{ transform: `translateY(${first * LIMITS.rowHeight}px)` }">
          <div v-for="(row, index) in visible" :id="`log-row-${first + index}`" :key="rowKey(row)" class="log-row" :class="{ 'is-selected': selected && rowKey(row) === rowKey(selected), 'has-gap': row.gap }" role="row" :aria-rowindex="offset + first + index + 1" :aria-selected="Boolean(selected && rowKey(row) === rowKey(selected))" @click="choose(first + index)" @dblclick="emit('inspect')">
            <span class="line-position" role="gridcell" :title="`记录 ${row.seq} · 字节偏移 ${row.offset}`">{{ history ? offset + first + index + 1 : `${row.seq}:${row.offset}` }}</span><time role="gridcell">{{ clockTime(row.time) }}</time><code role="gridcell"><template v-for="(part, i) in highlight(row.text || '', keyword)" :key="i"><mark v-if="part.match">{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></code>
          </div>
        </div>
      </div>
    </div>
    <button v-if="hasStream && !history && !follow && rows.length" class="return-tail button" @click="emit('update:follow', true); bottom()"><UiIcon name="follow" />回到最新</button>
  </div>
</template>
