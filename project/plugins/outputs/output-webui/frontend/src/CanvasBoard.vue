<script setup lang="ts">
import { computed, inject, ref, watch, nextTick, onMounted, onBeforeUnmount } from "vue";
import {
  VueFlow,
  useVueFlow,
  type Node,
  type NodeDragEvent,
  type ViewportTransform,
} from "@vue-flow/core";
import { NodeResizer, type OnResizeEnd } from "@vue-flow/node-resizer";
import { MiniMap } from "@vue-flow/minimap";
import Panel from "./Panel.vue";
import { AppContext, type Data } from "./api";
import { createViewportSaver, sameViewport } from "./viewportSave";
const ctx = inject(AppContext)!;
const page = computed(() =>
  ctx.state.value.pages.find((p: Data) => p.id === ctx.state.value.selected),
);
const nodes = ref<Node[]>([]);
const flow = useVueFlow();
let gestureRevision: number | undefined;
let manipulating = false,
  applyingViewport = false;
const pageId = page.value.id;
function syncNodes() {
  nodes.value = (page.value?.panels || [])
    .filter((p: Data) => !p.hidden)
    .map((p: Data) => ({
      id: p.id,
      type: "panel",
      position: { x: p.left, y: p.top },
      style: { width: `${p.panel_width}px`, height: `${p.panel_height}px` },
      data: { panel: p },
      dragHandle: ".panel-drag",
      selectable: false,
      selected: p.id === page.value.active_panel,
      draggable: !page.value.locked && !p.locked && page.value.tool !== "pan",
      zIndex: p.z_index,
    }));
}
watch(
  () =>
    JSON.stringify([
      page.value?.panels.map((p: Data) => [
        p.id,
        p.left,
        p.top,
        p.panel_width,
        p.panel_height,
        p.z_index,
        p.hidden,
        p.locked,
      ]),
      page.value?.active_panel,
      page.value?.locked,
      page.value?.tool,
    ]),
  () => {
    if (!manipulating) syncNodes();
  },
  { immediate: true },
);
const viewport = computed(() => ({
  x: page.value?.view_x || 0,
  y: page.value?.view_y || 0,
  zoom: page.value?.view_zoom || 1,
}));
async function syncViewport() {
  applyingViewport = true;
  try {
    await flow.setViewport(viewport.value);
  } finally {
    applyingViewport = false;
  }
}
watch(
  () => JSON.stringify(viewport.value),
  () => {
    if (!viewportSaver.busy) void syncViewport();
  },
);
function beginGesture() {
  manipulating = true;
  gestureRevision = ctx.state.value.revision;
}
async function commitLayout(layout: Data[]) {
  try {
    await ctx.command("layout.set", {
      page: page.value.id,
      revision: gestureRevision,
      layout,
    });
  } catch {
    /* Reconcile to the committed layout on a conflict. */
  } finally {
    manipulating = false;
    gestureRevision = undefined;
    syncNodes();
  }
}
async function dragged(event: NodeDragEvent) {
  await commitLayout(
    event.nodes.map((n) => ({
      id: n.id,
      left: Math.round(n.position.x),
      top: Math.round(n.position.y),
    })),
  );
}
async function resized(id: string, event: OnResizeEnd) {
  const p = event.params;
  await commitLayout([
    {
      id,
      left: Math.round(p.x),
      top: Math.round(p.y),
      panel_width: Math.round(p.width),
      panel_height: Math.round(p.height),
    },
  ]);
}
function pageViewport(p: Data) {
  return { x: p.view_x, y: p.view_y, zoom: p.view_zoom };
}
const viewportSaver = createViewportSaver({
  read: () => viewport.value,
  write: async (value, base) => {
    await ctx.command("page.set", {
      page: pageId,
      view_x: value.x,
      view_y: value.y,
      view_zoom: value.zoom,
    }, {
      // Own acknowledged saves may advance the revision. Only rebase while
      // the fields we are editing still match; the server also checks revision.
      guard: (state) => {
        const p = state.pages.find((p: Data) => p.id === pageId);
        return !!p && sameViewport(pageViewport(p), base);
      },
    });
  },
  reconcile: () => { void syncViewport(); },
  pending: (delta) => { ctx.deferredEdits.value += delta; },
});
// Vue Flow omits viewport-change-end when the transform did not change (for
// example pinching against a zoom limit, or pressing pan without moving).
let inputEndTimer: ReturnType<typeof setTimeout> | undefined;
function viewportInputEnd() {
  clearTimeout(inputEndTimer);
  // A microtask from a capture listener can run before D3's target listener.
  // Use a task so the start/change events of this input have already fired.
  inputEndTimer = setTimeout(() => {
    if (!applyingViewport && viewportSaver.busy)
      viewportSaver.end(flow.getViewport());
  }, 0);
}
onMounted(() => window.addEventListener("mouseup", viewportInputEnd, true));
onBeforeUnmount(() => {
  window.removeEventListener("mouseup", viewportInputEnd, true);
  clearTimeout(inputEndTimer);
  viewportSaver.dispose();
});
function viewportStart() {
  if (!applyingViewport) viewportSaver.begin();
}
function viewportChange(value: ViewportTransform) {
  if (!applyingViewport) viewportSaver.change(value);
}
function viewportEnd(value: ViewportTransform) {
  if (!applyingViewport) viewportSaver.end(value);
}
async function moveViewport(move: () => Promise<unknown>) {
  viewportSaver.begin();
  applyingViewport = true;
  try {
    await move();
  } finally {
    applyingViewport = false;
    viewportSaver.end(flow.getViewport());
  }
}
async function fit(id?: string) {
  await moveViewport(() => flow.fitView({
    nodes: id ? [id] : undefined,
    padding: 0.12,
    minZoom: 0.2,
    maxZoom: 1.0,
    duration: 0,
  }));
}
async function zoom(factor: number) {
  const current = flow.getViewport(), size = flow.dimensions.value;
  const next = Math.min(2, Math.max(0.2, current.zoom * factor));
  await moveViewport(() => flow.setViewport({
    x: size.width / 2 - ((size.width / 2 - current.x) * next) / current.zoom,
    y: size.height / 2 - ((size.height / 2 - current.y) * next) / current.zoom,
    zoom: next,
  }));
}
async function reset() {
  await moveViewport(() => flow.setViewport({ x: 24, y: 24, zoom: 1 }));
}
async function initialized() {
  await nextTick();
  await syncViewport();
}
defineExpose({ fit, zoom, reset });
</script>
<template>
  <VueFlow
    v-model:nodes="nodes"
    :edges="[]"
    class="canvas-board"
    :class="{ 'with-grid': page.show_grid }"
    :default-viewport="viewport"
    :min-zoom="0.2"
    :max-zoom="2"
    :nodes-connectable="false"
    :elements-selectable="false"
    :select-nodes-on-drag="false"
    :delete-key-code="null"
    :selection-key-code="null"
    :snap-to-grid="page.snap"
    :snap-grid="[8, 8]"
    :pan-on-drag="page.tool === 'pan' ? true : [1, 2]"
    :pan-on-scroll="true"
    :zoom-on-scroll="false"
    :zoom-on-pinch="true"
    :zoom-on-double-click="false"
    :prevent-scrolling="true"
    :elevate-nodes-on-select="false"
    :auto-pan-on-node-drag="false"
    @init="initialized"
    @node-drag-start="beginGesture"
    @node-drag-stop="dragged"
    @viewport-change-start="viewportStart"
    @viewport-change="viewportChange"
    @viewport-change-end="viewportEnd"
    @wheel.capture="viewportInputEnd"
    @pane-click="ctx.selectPanel(null)"
  >
    <template #node-panel="{ id }">
      <NodeResizer
        :node-id="id"
        :is-visible="
          page.active_panel === id &&
          !page.locked &&
          !page.panels.find((p: Data) => p.id === id)?.locked
        "
        :min-width="320"
        :min-height="220"
        :max-width="4000"
        :max-height="4000"
        color="var(--accent)"
        @resize-start="beginGesture"
        @resize-end="(event: OnResizeEnd) => resized(id, event)"
      />
      <Panel :id="id" />
    </template>
    <MiniMap
      v-if="page.show_minimap"
      :pannable="true"
      :zoomable="true"
      :node-color="'var(--canvas-node)'"
      :mask-color="'var(--canvas-mask)'"
    />
  </VueFlow>
</template>
