<script setup lang="ts">
import { computed, ref } from 'vue';
import type { ImportJob, StreamInfo } from '../types';
import { streamName } from '../display';
import UiIcon from './UiIcon.vue';
const props = defineProps<{ streams: StreamInfo[]; imports: ImportJob[]; selected: string; uploadBusy: boolean; importEnabled: boolean; connected: boolean }>();
defineEmits<{ select: [id: string]; upload: []; close: [] }>();
const search = ref('');
const groups = computed(() => [
  { name: '实时来源', kind: 'stream', streams: props.streams.filter(s => !props.imports.some(i => i.stream === s.id)) },
  { name: '导入文件', kind: 'file', streams: props.streams.filter(s => props.imports.some(i => i.stream === s.id)) },
].map(g => ({ ...g, streams: g.streams.filter(s => `${streamName(s)} ${s.owner}`.toLowerCase().includes(search.value.toLowerCase())) })));
function status(stream: StreamInfo) {
  const job = props.imports.find(i => i.stream === stream.id);
  if (job) return job.state === 'complete' ? '文件已就绪' : job.state === 'failed' ? '导入失败' : '正在导入';
  return stream.writer_active ? '采集中' : '可读缓存';
}
</script>
<template>
  <div class="source-heading"><h2>来源</h2><span class="count">{{ streams.length }}</span><button class="icon-button mobile-only" aria-label="关闭来源" @click="$emit('close')"><UiIcon name="close" /></button></div>
  <div v-if="streams.length > 3" class="source-search"><UiIcon name="search" :size="14" /><input v-model="search" aria-label="查找来源" placeholder="查找来源…" /></div>
  <nav class="source-scroll" aria-label="日志来源">
    <template v-for="group in groups" :key="group.kind">
      <div v-if="group.streams.length" class="source-group-label">{{ group.name }}<span>{{ group.streams.length }}</span></div>
      <button v-for="item in group.streams" :key="item.id" class="source-item" :class="{ selected: item.id === selected }" :aria-current="item.id === selected ? 'true' : undefined" :title="`${streamName(item)}\n${item.id}`" @click="$emit('select', item.id)">
        <UiIcon :name="group.kind" /><span class="source-text"><strong>{{ streamName(item) }}</strong><small><i :class="['status-dot', { live: group.kind === 'stream' && item.writer_active, failed: status(item) === '导入失败' }]" />{{ status(item) }}</small></span>
      </button>
    </template>
    <p v-if="!streams.length" class="source-empty">还没有日志来源<br /><span>导入文件，或等待采集连接。</span></p>
    <p v-else-if="groups.every(g => !g.streams.length)" class="source-empty">没有匹配的来源</p>
  </nav>
  <div class="source-bottom"><button class="button import-source" :disabled="uploadBusy || !importEnabled || !connected" @click="$emit('upload')"><UiIcon name="upload" />{{ uploadBusy ? '上传中…' : '导入日志文件' }}</button><small>{{ importEnabled ? '单个文件最多 64 MiB' : '导入需要 TCP supervisor 实例' }}</small></div>
</template>
