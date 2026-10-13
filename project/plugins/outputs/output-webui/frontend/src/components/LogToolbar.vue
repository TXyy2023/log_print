<script setup lang="ts">
import { ref } from 'vue';
import UiIcon from './UiIcon.vue';
defineProps<{ keyword: string; onlyMatches: boolean; hasStream: boolean; matches: number; matchIndex: number; history: boolean }>();
defineEmits<{ 'update:keyword': [value: string]; 'update:onlyMatches': [value: boolean]; navigate: [direction: number] }>();
const input = ref<HTMLInputElement>();
defineExpose({ focus: () => { input.value?.focus(); input.value?.select(); } });
</script>
<template>
  <div class="query-toolbar" role="search" aria-label="日志搜索">
    <div class="query-input"><UiIcon name="search" /><input ref="input" :value="keyword" aria-label="关键词" placeholder="查找日志内容…" :disabled="!hasStream" @input="$emit('update:keyword', ($event.target as HTMLInputElement).value)" @keydown.enter.prevent="$emit('navigate', $event.shiftKey ? -1 : 1)" @keydown.esc.prevent="$emit('update:keyword', '')" /><button v-if="keyword" class="icon-button" aria-label="清除关键词" @click="$emit('update:keyword', '')"><UiIcon name="close" :size="14" /></button><kbd v-else class="search-shortcut">⌘ / Ctrl F</kbd></div>
    <div class="search-controls"><span v-if="keyword" class="match-counter" aria-live="polite">{{ matchIndex >= 0 ? matchIndex + 1 : 0 }} / {{ matches }}</span><div class="match-arrows"><button class="icon-button" :disabled="!matches" aria-label="上一处匹配" title="上一处匹配 · Shift Enter" @click="$emit('navigate', -1)"><UiIcon name="up" /></button><button class="icon-button" :disabled="!matches" aria-label="下一处匹配" title="下一处匹配 · Enter" @click="$emit('navigate', 1)"><UiIcon name="down" /></button></div><label class="filter-toggle"><input type="checkbox" :checked="onlyMatches" :disabled="!hasStream" @change="$emit('update:onlyMatches', ($event.target as HTMLInputElement).checked)" /><span>仅显示匹配</span></label></div>
    <span class="query-scope">{{ history ? '搜索可用历史' : '查找实时缓存' }}</span>
  </div>
</template>
