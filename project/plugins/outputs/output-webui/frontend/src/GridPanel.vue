<script setup lang="ts">
import { onMounted, nextTick } from "vue";
import { useGridStack, useGridStackItem } from "gridstack/dist/vue";
import Panel from "./Panel.vue";
defineProps<{ id: string }>();
const stack = useGridStack(),
  item = useGridStackItem();
onMounted(async () => {
  await nextTick();
  if (item.node?.el) stack.grid?.refreshDragHandles(item.node.el);
});
</script>
<template><Panel :id="id" /></template>
