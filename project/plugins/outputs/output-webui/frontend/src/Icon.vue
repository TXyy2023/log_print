<script setup lang="ts">
import { computed } from "vue";
import {
  Plus,
  Search,
  Close,
  Setting,
  DataLine,
  Document,
  VideoPause,
  VideoPlay,
  FullScreen,
  ZoomIn,
  ZoomOut,
  Pointer,
  Rank,
  Lock,
  Unlock,
  View,
  Hide,
  CopyDocument,
  Delete,
  Refresh,
  MoreFilled,
  ArrowLeft,
  ArrowRight,
  Check,
  Warning,
  FolderOpened,
  Operation,
  Grid,
  Connection,
  Menu,
} from "@element-plus/icons-vue";
const props = defineProps<{ name: string; size?: number }>();
const icons: Record<string, any> = {
  plus: Plus,
  search: Search,
  close: Close,
  settings: Setting,
  curve: DataLine,
  log: Document,
  pause: VideoPause,
  play: VideoPlay,
  fit: FullScreen,
  zoomIn: ZoomIn,
  zoomOut: ZoomOut,
  select: Pointer,
  pan: Rank,
  lock: Lock,
  unlock: Unlock,
  view: View,
  hide: Hide,
  copy: CopyDocument,
  delete: Delete,
  refresh: Refresh,
  more: MoreFilled,
  back: ArrowLeft,
  next: ArrowRight,
  check: Check,
  warning: Warning,
  page: FolderOpened,
  inspector: Operation,
  grid: Grid,
  stream: Connection,
  menu: Menu,
};
const icon = computed(() => icons[props.name] || Document);
</script>
<template>
  <component
    :is="icon"
    class="ui-icon"
    :style="{ width: `${size || 16}px`, height: `${size || 16}px` }"
    aria-hidden="true"
  />
</template>
