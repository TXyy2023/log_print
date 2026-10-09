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
import { available, compact, snapMove, type Rect } from "./layout";
import { clone, equal } from "./panelDraft";
const ctx = inject(AppContext)!;
const page = computed(() =>
  ctx.state.value.pages.find((p: Data) => p.id === ctx.state.value.selected),
);
const nodes = ref<Node[]>([]);
const flow = useVueFlow();
let gestureBase: Data[] = [];
const root = ref<HTMLElement>();
const preview = ref<Rect>();
const layoutError = ref("");
const previewStyle = computed(() => {
  const p = preview.value, v = flow.viewport.value;
  return p ? { left: `${p.left * v.zoom + v.x}px`, top: `${p.top * v.zoom + v.y}px`, width: `${p.panel_width * v.zoom}px`, height: `${p.panel_height * v.zoom}px` } : {};
});
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
  gestureBase = clone(page.value.panels);
  layoutError.value = "";
}
async function commitLayout(layout: Data[]) {
  const original = clone(gestureBase);
  try {
    await ctx.command("layout.set", {
      page: pageId,
      layout,
    }, {
      localError: true, editKey: `layout:${pageId}`, retryConflict: true,
      guard: (state) => {
        const current = state.pages.find((p: Data) => p.id === pageId);
        if (!current || current.locked) return false;
        const result = current.panels.map((p: Data) => ({ ...p, ...layout.find(item => item.id === p.id) }));
        return layout.every(item => {
          const p = current.panels.find((p: Data) => p.id === item.id);
          const base = original.find(p => p.id === item.id);
          return p && base && !p.locked && Object.keys(item).every(key => key === 'id' || equal(p[key], base[key])) &&
            (current.allow_overlap || available({ ...p, ...item }, result));
        });
      },
    });
  } catch (e) {
    layoutError.value = e instanceof Error ? e.message : String(e);
  } finally {
    manipulating = false;
    preview.value = undefined;
    syncNodes();
  }
}
function dragging(event: NodeDragEvent) {
  const n = event.node;
  const panel = page.value.panels.find((p: Data) => p.id === n.id);
  const rect = snapMove({ ...panel, left: Math.round(n.position.x), top: Math.round(n.position.y) },
    page.value.panels, 8 / flow.getViewport().zoom, page.value.snap, page.value.allow_overlap);
  n.position = { x: rect.left, y: rect.top };
  preview.value = rect;
}
function canResize(id: string, params: { x: number; y: number; width: number; height: number }) {
  const rect = { id, left: params.x, top: params.y, panel_width: params.width, panel_height: params.height };
  const valid = page.value.allow_overlap || available(rect, page.value.panels);
  if (valid) preview.value = rect;
  return valid;
}
async function dragged(event: NodeDragEvent) {
  dragging(event);
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
  // Align the released edges without changing the opposite resize anchor.
  const original = gestureBase.find(p => p.id === id)!;
  const rect = { id, left: Math.round(p.x), top: Math.round(p.y), panel_width: Math.round(p.width), panel_height: Math.round(p.height) };
  const tolerance = 8 / flow.getViewport().zoom;
  if (page.value.snap) {
    const right = rect.left + rect.panel_width, bottom = rect.top + rect.panel_height;
    for (const other of page.value.panels.filter((p: Data) => p.id !== id && !p.hidden)) {
      const edgesX = [other.left - 8, other.left + other.panel_width, other.left, other.left + other.panel_width + 8];
      const edgesY = [other.top - 8, other.top + other.panel_height, other.top, other.top + other.panel_height + 8];
      for (const x of edgesX) {
        const proposed = { ...rect };
        if (rect.left !== original.left && Math.abs(x - rect.left) <= tolerance) { proposed.left = x; proposed.panel_width = right - x; }
        else if (rect.left === original.left && Math.abs(x - right) <= tolerance) proposed.panel_width = x - rect.left;
        if (proposed.panel_width >= 320 && proposed.panel_width <= 4000 && (page.value.allow_overlap || available(proposed, page.value.panels))) Object.assign(rect, proposed);
      }
      for (const y of edgesY) {
        const proposed = { ...rect };
        if (rect.top !== original.top && Math.abs(y - rect.top) <= tolerance) { proposed.top = y; proposed.panel_height = bottom - y; }
        else if (rect.top === original.top && Math.abs(y - bottom) <= tolerance) proposed.panel_height = y - rect.top;
        if (proposed.panel_height >= 220 && proposed.panel_height <= 4000 && (page.value.allow_overlap || available(proposed, page.value.panels))) Object.assign(rect, proposed);
      }
    }
  }
  await commitLayout([
    {
      ...rect,
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
function availableWidth() { return Math.max(320, (flow.dimensions.value.width - 48) / flow.getViewport().zoom); }
async function compactLayout() {
  if (page.value.locked) return;
  beginGesture();
  const layout = compact(page.value.panels, availableWidth()).filter(p => !p.locked)
    .map(({ id, left, top }) => ({ id, left, top }));
  await commitLayout(layout);
  if (!layoutError.value) await fit();
}
function wheel(event: WheelEvent) {
  const target = event.target as HTMLElement;
  const onPanel = !!target.closest('.panel');
  const panPanel = onPanel && (page.value.tool === 'pan' || !!target.closest('.panel-header'));
  if (!event.ctrlKey && !panPanel) { viewportInputEnd(); return; }
  event.preventDefault();
  event.stopImmediatePropagation();
  const current = flow.getViewport();
  const bounds = root.value!.getBoundingClientRect();
  const x = event.clientX - bounds.left, y = event.clientY - bounds.top;
  const unit = event.deltaMode === 1 ? 20 : event.deltaMode === 2 ? bounds.height : 1;
  const next = event.ctrlKey ? Math.min(2, Math.max(0.2, current.zoom * 2 ** (-event.deltaY * unit * 0.01))) : current.zoom;
  void moveViewport(() => flow.setViewport({
    zoom: next,
    x: event.ctrlKey ? x - (x - current.x) * next / current.zoom : current.x - event.deltaX * unit,
    y: event.ctrlKey ? y - (y - current.y) * next / current.zoom : current.y - event.deltaY * unit,
  }));
}
defineExpose({ fit, zoom, reset, compactLayout, availableWidth });
</script>
<template>
  <div ref="root" class="canvas-shell" @wheel.capture="wheel">
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
    @node-drag="dragging"
    @node-drag-stop="dragged"
    @viewport-change-start="viewportStart"
    @viewport-change="viewportChange"
    @viewport-change-end="viewportEnd"
    @pane-click="ctx.selectPanel(null)"
  >
    <template #node-panel="{ id }">
      <NodeResizer
        :node-id="id"
        :should-resize="(_event, params) => canResize(id, params)"
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
  <div v-if="preview" class="layout-preview" :style="previewStyle"><i class="guide-horizontal"></i><i class="guide-vertical"></i></div>
  <div v-if="layoutError" class="layout-error" role="alert">{{ layoutError }}<button class="text-button" @click="layoutError = ''; ctx.clearEditError(`layout:${pageId}`)">关闭</button></div>
  </div>
</template>
