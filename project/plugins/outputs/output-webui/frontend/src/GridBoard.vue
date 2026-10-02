<script setup lang="ts">
import { computed, inject, markRaw, ref, shallowRef, watch } from "vue";
import { GridStack, type GridStackOptions } from "gridstack/dist/vue";
import type { GridStack as CoreGridStack, GridStackNode } from "gridstack";
import GridPanel from "./GridPanel.vue";
import { AppContext, type Data } from "./api";
const ctx = inject(AppContext)!;
const page = computed(() =>
  ctx.state.value.pages.find((p: Data) => p.id === ctx.state.value.selected),
);
const grid = ref<{ getGrid: () => CoreGridStack | null }>();
const components = { Panel: markRaw(GridPanel) };
const options = shallowRef<GridStackOptions>({});
let revision: number | undefined;
function configure() {
  options.value = {
    column: 12,
    mode: "float",
    cellHeight: 72,
    margin: 8,
    handle: ".panel-drag",
    disableDrag: page.value.locked,
    disableResize: page.value.locked,
    resizable: { handles: "se,sw" },
    children: page.value.panels
      .filter((p: Data) => !p.hidden)
      .map((p: Data) => ({
        id: p.id,
        x: p.x,
        y: p.y,
        w: p.w,
        h: p.h,
        noMove: p.locked,
        noResize: p.locked,
        component: "Panel",
        props: { id: p.id },
      })),
  };
}
watch(
  () =>
    JSON.stringify([
      page.value?.panels.map((p: Data) => [
        p.id,
        p.x,
        p.y,
        p.w,
        p.h,
        p.hidden,
        p.locked,
      ]),
      page.value?.locked,
    ]),
  configure,
  { immediate: true },
);
async function save() {
  try {
    await ctx.command("layout.set", {
      page: page.value.id,
      revision,
      layout: (grid.value?.getGrid()?.engine.nodes || []).map(
        (n: GridStackNode) => ({ id: n.id, x: n.x, y: n.y, w: n.w, h: n.h }),
      ),
    });
  } catch {
    configure();
  } finally {
    revision = undefined;
  }
}
</script>
<template>
  <div class="grid-board">
    <GridStack
      ref="grid"
      :options="options"
      :components="components"
      @dragstart="revision = ctx.state.value.revision"
      @resizestart="revision = ctx.state.value.revision"
      @dragstop="save"
      @resizestop="save"
    />
  </div>
</template>
