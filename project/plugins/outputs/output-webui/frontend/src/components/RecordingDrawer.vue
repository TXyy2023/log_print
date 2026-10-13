<script setup lang="ts">
import { onMounted, ref } from 'vue';
import type { Recording, StreamInfo } from '../types';
import { fileSize, hasGaps, recordingLabel, streamName } from '../display';
import UiIcon from './UiIcon.vue';
defineProps<{ recordings: Recording[]; streams: StreamInfo[]; busy: boolean; feedback: string }>();
const emit = defineEmits<{ close: []; stop: [id: string]; copy: [text: string] }>();
const dialog = ref<HTMLDialogElement>();
onMounted(() => dialog.value?.showModal());
</script>
<template>
  <dialog ref="dialog" class="recording-drawer" aria-labelledby="recordings-title" @cancel.prevent="emit('close')" @click="$event.target === dialog && emit('close')">
    <header class="panel-title"><h2 id="recordings-title">保存记录 <span class="count">{{ recordings.length }}</span></h2><button class="icon-button" aria-label="关闭保存记录" @click="emit('close')"><UiIcon name="close" /></button></header>
    <p class="drawer-description">保存包含原始日志，暂停和筛选不会影响文件内容。</p>
    <div v-if="!recordings.length" class="drawer-empty"><UiIcon name="save" :size="28" /><h3>还没有保存记录</h3><p>选择来源后，使用“保存日志”开始持续记录。</p></div>
    <div v-else class="recording-list"><article v-for="record in recordings" :key="record.id" class="recording"><div class="recording-main"><span :class="['status-dot', `record-${record.state}`]" /><div><h3>{{ streamName(streams.find(s => s.id === record.stream)) }}</h3><p>{{ record.mode === 'now' ? '从现在开始' : '从最早可用数据' }}<span>·</span>{{ fileSize(record.bytes) }}<span>·</span><strong>{{ recordingLabel(record.state) }}</strong></p></div><a v-if="record.state === 'stopped'" class="button" :href="`/api/recordings/${record.id}/download`" download><UiIcon name="download" />下载 .log</a><button v-else-if="record.state !== 'failed'" class="button" :disabled="busy" @click="emit('stop', record.id)"><UiIcon name="stop" :size="14" />停止并保存</button></div><p v-if="record.error" class="inline-error" role="alert">{{ record.error }}</p><p v-if="record.writer?.sqlite?.incomplete || hasGaps(record.coverage)" class="inline-warning">仅保存可获取的数据；记录范围存在缺口。</p><details class="saved-path"><summary>文件位置</summary><div><code>{{ record.path }}</code><button class="text-button" @click="emit('copy', record.path)"><UiIcon name="copy" :size="14" />复制路径</button></div></details></article></div>
    <footer class="drawer-footer"><span v-if="feedback" role="status">{{ feedback }}</span><span v-else>文件保存在本机。停止并提交后可下载。</span></footer>
  </dialog>
</template>
