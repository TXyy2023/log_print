<script setup lang="ts">
import type { LogRow } from '../types';
import { clockTime, rowKey } from '../display';
import UiIcon from './UiIcon.vue';
const props = defineProps<{ row: LogRow; streamName: string; contextRows: LogRow[]; busy: boolean; error: string; overlay: boolean }>();
const emit = defineEmits<{ close: []; context: []; copy: [text: string] }>();
async function copy() { emit('copy', props.row.text); }
</script>
<template>
  <aside class="record-inspector" :role="overlay ? 'dialog' : 'complementary'" :aria-modal="overlay || undefined" aria-labelledby="detail-title" tabindex="-1">
    <div class="panel-title"><h2 id="detail-title">日志详情</h2><button class="icon-button" aria-label="关闭详情" @click="$emit('close')"><UiIcon name="close" /></button></div>
    <div class="inspector-body"><div class="detail-kicker">选中的日志</div><div class="detail-copy"><span>原始文本</span><button class="text-button" @click="copy"><UiIcon name="copy" :size="14" />复制</button></div><pre class="raw-text">{{ row.text }}</pre>
      <dl class="metadata"><div><dt>来源</dt><dd>{{ streamName }}</dd></div><div><dt>采集时间</dt><dd>{{ clockTime(row.time) }}</dd></div><div><dt>通道</dt><dd>{{ row.channel || '未指定' }}</dd></div><div><dt>记录序号</dt><dd>{{ row.seq }}</dd></div><div><dt>字节偏移</dt><dd>{{ row.offset }}</dd></div><div v-if="row.partial"><dt>行状态</dt><dd>未结束的行 / 分段内容</dd></div></dl>
      <div class="context-heading"><h3>前后上下文</h3><button class="text-button" :disabled="busy" @click="$emit('context')"><UiIcon name="context" :size="14" />{{ busy ? '读取中…' : contextRows.length ? '重新读取' : '读取前后 8 行' }}</button></div>
      <p v-if="error" class="inline-error" role="alert">{{ error }}</p><p v-else-if="!contextRows.length" class="muted context-hint">从可读历史中定位这条日志，保留筛选之外的前后内容。</p>
      <div v-else class="context-lines"><div v-for="item in contextRows" :key="rowKey(item)" :class="{ target: item.context_target }"><span>{{ item.seq }}</span><code>{{ item.text }}</code></div></div>
      <details class="identity"><summary>来源标识</summary><dl><dt>Stream</dt><dd>{{ row.stream }}</dd><dt>Epoch</dt><dd>{{ row.epoch }}</dd></dl></details>
    </div>
  </aside>
</template>
