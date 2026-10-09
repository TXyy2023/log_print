<script setup lang="ts">
import {
  computed,
  inject,
  onMounted,
  onBeforeUnmount,
  ref,
  shallowRef,
  watch,
  nextTick,
} from "vue";
import { AgGridVue } from "ag-grid-vue3";
import Icon from "./Icon.vue";
import {
  AllCommunityModule,
  ModuleRegistry,
  themeQuartz,
  type GridApi,
  type ColDef,
  type ColumnState,
} from "ag-grid-community";
import * as echarts from "echarts/core";
import { LineChart } from "echarts/charts";
import {
  GridComponent,
  TooltipComponent,
  LegendComponent,
  DataZoomComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import { ElMessage } from "element-plus";
import { AppContext, bindingValue, type Data } from "./api";
import { coveragePresentation } from "./coverage";
import { clone, patchFrom, conflictingFields, fieldLabels } from "./panelDraft";
import { place } from "./layout";
const menuButton = ref<HTMLElement>(), cancelRemove = ref<HTMLButtonElement>();
const removeOpen = ref(false), removing = ref(false), removeError = ref("");
const settingsBase = ref<Data>({}), settingsError = ref(""), settingsSaving = ref(false);
const settingsFields = ["title", "streams", "channels", "regex", "format", "follow", "metadata", "columns", "legend", "y_min", "y_max", "time_from", "time_end"];
const normalizeSettings = (p: Data): Data => ({ ...clone(p), streams: p.streams.map((s: Data) => bindingValue(s)) });
ModuleRegistry.registerModules([AllCommunityModule]);
echarts.use([
  LineChart,
  GridComponent,
  TooltipComponent,
  LegendComponent,
  DataZoomComponent,
  CanvasRenderer,
]);
const props = defineProps<{ id: string }>(),
  ctx = inject(AppContext)!;
// Async work belongs to this mounted panel, even after the selected page changes.
const pageId = ctx.state.value.selected,
  panelId = props.id;
let disposed = false;
const page = computed(() =>
  ctx.state.value.pages.find((p: Data) => p.id === pageId),
);
const panel = computed<Data>(
  () => page.value?.panels.find((p: Data) => p.id === panelId) || {},
);
function active() {
  return (
    !disposed &&
    ctx.state.value.selected === pageId &&
    panel.value.id === panelId &&
    !panel.value.hidden
  );
}
const filterText = ref("");
let filterRevision: number | undefined;
watch(
  () => panel.value.text,
  (text) => {
    filterText.value = text || "";
  },
  { immediate: true },
);
const rows = ref<Data[]>([]),
  result = ref<Data>({}),
  settings = ref(false),
  seriesDialog = ref(false),
  form = ref<Data>({}),
  seriesForm = ref<Data>({}),
  chartElement = ref<HTMLElement>(),
  working = ref(false);
let zoomTimer: ReturnType<typeof setTimeout> | undefined,
  columnTimer: ReturnType<typeof setTimeout> | undefined;
let loadedCurveQuery: string | undefined;
let textCommit: Promise<unknown> = Promise.resolve();
function changeText(text: string) {
  textCommit = set({
    text,
    revision: filterRevision ?? ctx.state.value.revision,
  });
  void textCommit.catch(() => {});
  return textCommit;
}
let chart: echarts.ECharts | undefined,
  gridApi: GridApi | undefined,
  timer: ReturnType<typeof setInterval> | undefined,
  observer: ResizeObserver | undefined,
  updating = false,
  fetching = false;
const theme = computed(() =>
  themeQuartz.withParams({
    browserColorScheme: page.value?.theme === "dark" ? "dark" : "light",
    backgroundColor: page.value?.theme === "light" ? "#fff" : "#1b1e24",
    foregroundColor: page.value?.theme === "light" ? "#303640" : "#d9dce3",
    borderColor: page.value?.theme === "light" ? "#e7e9ed" : "#30343d",
    fontSize: panel.value.font_size || 12,
    fontFamily: 'SFMono-Regular, Consolas, "Liberation Mono", monospace',
    headerFontFamily:
      '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
    headerFontSize: 11,
    headerFontWeight: 500,
    rowBorder: false,
    wrapperBorder: false,
    headerHeight: 30,
    cellHorizontalPadding: 12,
    headerBackgroundColor:
      page.value?.theme === "light" ? "#f8f9fb" : "#22262e",
  }),
);
const allColumns = [
  "time",
  "stream",
  "channel",
  "seq",
  "offset",
  "key",
  "text",
  "epoch",
  "source_ts_ns",
  "source_seq",
  "upstream",
  "upstream_epochs",
];
const columns = shallowRef<ColDef[]>([]);
watch(
  () =>
    JSON.stringify([
      panel.value.columns,
      panel.value.metadata,
      panel.value.column_state,
      panel.value.format,
    ]),
  () => {
    const states = panel.value.column_state || [];
    const ordered = [...allColumns].sort((a, b) => {
      const rank = (field: string) => {
        const i = states.findIndex((c: Data) => c.colId === field);
        return i < 0 ? 999 : i;
      };
      return rank(a) - rank(b);
    });
    columns.value = ordered.map((field) => {
      const stored = states.find((c: Data) => c.colId === field) || {};
      return {
        field,
        hide:
          !(panel.value.columns || []).includes(field) &&
          !(
            panel.value.metadata &&
            [
              "seq",
              "offset",
              "key",
              "source_ts_ns",
              "source_seq",
              "upstream",
              "upstream_epochs",
            ].includes(field)
          ),
        headerName:
          (
            {
              time: "观测时间",
              stream: "流",
              channel: "通道",
              seq: "序号",
              offset: "字节偏移",
              text: "内容",
            } as Data
          )[field] || field,
        flex: Object.hasOwn(stored, "flex")
          ? stored.flex
          : field === "text"
            ? 3
            : undefined,
        width:
          stored.width ??
          (field === "text" ? 500 : field === "stream" ? 180 : 140),
        sort: stored.sort,
        sortIndex: stored.sortIndex,
        resizable: true,
        sortable: true,
        comparator: (a: any, b: any) =>
          ["seq", "time", "source_ts_ns", "source_seq"].includes(field) &&
          /^\d+$/.test(String(a)) &&
          /^\d+$/.test(String(b))
            ? BigInt(a) === BigInt(b)
              ? 0
              : BigInt(a) > BigInt(b)
                ? 1
                : -1
            : String(a ?? "").localeCompare(String(b ?? "")),
        valueFormatter: (params) =>
          field === "time"
            ? new Date(Number(params.value)).toLocaleTimeString("zh-CN", {
                hour12: false,
              }) +
              "." +
              String(new Date(Number(params.value)).getMilliseconds()).padStart(
                3,
                "0",
              )
            : field === "stream"
              ? ctx.state.value.streams.find((s: Data) => s.id === params.value)
                  ?.alias || String(params.value || "").slice(0, 8)
              : field === "text" && panel.value.format === "hex"
                ? (params.data?.hex ?? params.data?.text)
                : typeof params.value === "object"
                  ? JSON.stringify(params.value)
                  : String(params.value ?? ""),
      };
    });
  },
  { immediate: true },
);
const streamOptions = computed(() =>
  ctx.state.value.streams.map((s: Data) => ({
    value: bindingValue(
      s.alias
        ? { owner: s.owner, alias: s.alias }
        : { owner: s.owner, alias: null, stream: s.id, epoch: s.epoch },
    ),
    label: s.alias || `${s.owner} / ${s.id.slice(0, 8)}`,
  })),
);
const coverage = computed(() =>
  panel.value.mode === "history" ? result.value.status?.coverage : undefined,
);
const coverageStatus = computed(() =>
  coveragePresentation(panel.value.mode, ctx.state.value, coverage.value),
);
const waiting = computed(
  () => result.value.waiting === true || result.value.status?.waiting === true,
);
const sourceLabel = computed(() => {
  const bindings = panel.value.streams || [];
  return bindings.length
    ? bindings.map((s: Data) => s.alias || s.owner || "动态流").join(" · ")
    : "全部来源";
});
const queryRunning = computed(() => result.value.status?.state === "running");
async function set(args: Data) {
  if (!active()) return;
  await ctx.command("panel.set", {
    ...args,
    page: pageId,
    panel: panelId,
  });
}
async function update() {
  if (fetching || !active()) return;
  fetching = true;
  try {
    const p = panel.value;
    let data: Data;
    if (p.paused) {
      data = await ctx.command("panel.data", {
        page: pageId,
        panel: panelId,
      });
      if (!active()) return;
      rows.value = data.rows || [];
    } else if (p.mode === "history" && p.query) {
      data = await ctx.command("query.get", {
        query: p.query,
        offset: p.offset || 0,
      });
      if (!active()) return;
      rows.value = data.rows || [];
      if (p.kind === "curve" && data.status?.state === "complete") {
        if (loadedCurveQuery !== p.query) {
          const points = [...(data.rows || [])];
          let offset = data.next;
          while (offset < data.total && points.length < 2000) {
            const batch = await ctx.command("query.get", {
              query: p.query,
              offset,
            });
            if (!active()) return;
            points.push(...batch.rows);
            if (batch.next <= offset) break;
            offset = batch.next;
          }
          data.rows = points;
          loadedCurveQuery = p.query;
        } else {
          data.rows = result.value.rows || [];
        }
      }
    } else if (p.mode === "history") {
      rows.value = [];
      return;
    } else {
      loadedCurveQuery = undefined;
      data = await ctx.command("panel.data", {
        page: pageId,
        panel: panelId,
      });
      if (!active()) return;
      rows.value = data.rows || [];
    }
    result.value = data;
    if (panel.value.kind === "curve") draw(data);
    if (
      !p.paused &&
      panel.value.follow &&
      p.mode === "live" &&
      rows.value.length
    ) {
      await nextTick();
      if (!active()) return;
      const count = gridApi?.getDisplayedRowCount() || 0;
      if (count) gridApi?.ensureIndexVisible(count - 1, "bottom");
    }
  } catch (e) {
    if (active()) result.value = { error: String(e) };
  } finally {
    fetching = false;
  }
}
function draw(data: Data) {
  if (!active() || !chartElement.value) return;
  if (!chart) {
    chart = echarts.init(chartElement.value);
    chart.on(
      "legendselectchanged",
      (event: any) =>
        void set({ legend_selected: event.selected }).catch(() => {}),
    );
    chart.on("datazoom", (event: any) => {
      if (updating) return;
      const zoom = event.batch?.[0] || event;
      const revision = ctx.state.value.revision;
      if (zoomTimer) clearTimeout(zoomTimer);
      zoomTimer = setTimeout(
        () =>
          void set({
            revision,
            zoom_start: zoom.start,
            zoom_end: zoom.end,
          }).catch(() => {}),
        200,
      );
    });
  }
  updating = true;
  const history = panel.value.mode === "history";
  const series = history
    ? (panel.value.series || []).map((s: Data) => ({
        id: s.id,
        name: s.name,
        points: (data.rows || []).filter((p: Data) => p.series === s.id),
      }))
    : data.series || [];
  chart.setOption(
    {
      animation: false,
      backgroundColor: "transparent",
      textStyle: {
        color: page.value.theme === "light" ? "#727985" : "#9ca4b3",
        fontFamily: "-apple-system, BlinkMacSystemFont, sans-serif",
        fontSize: 11,
      },
      tooltip: { trigger: "axis", confine: true },
      legend: {
        show: panel.value.legend,
        selected: panel.value.legend_selected || {},
        top: 4,
        textStyle: {
          color: page.value.theme === "light" ? "#626b79" : "#afb7c5",
          fontSize: 11,
        },
      },
      grid: { left: 48, right: 20, top: 30, bottom: 48 },
      xAxis: {
        type: "time",
        axisLine: {
          lineStyle: {
            color: page.value.theme === "light" ? "#dfe3e9" : "#383d48",
          },
        },
        axisTick: { show: false },
        axisLabel: {
          hideOverlap: true,
          color: page.value.theme === "light" ? "#727985" : "#9ca4b3",
        },
      },
      yAxis: {
        type: "value",
        splitNumber: 3,
        axisLabel: {
          hideOverlap: true,
          color: page.value.theme === "light" ? "#727985" : "#9ca4b3",
        },
        min: panel.value.y_min ?? null,
        max: panel.value.y_max ?? null,
        splitLine: {
          lineStyle: {
            color: page.value.theme === "light" ? "#eef0f4" : "#2c3039",
            type: "dashed",
          },
        },
      },
      dataZoom: [
        {
          type: "inside",
          start: panel.value.zoom_start ?? 0,
          end: panel.value.zoom_end ?? 100,
        },
        {
          type: "slider",
          borderColor: page.value.theme === "light" ? "#dde3ed" : "#343c49",
          backgroundColor: page.value.theme === "light" ? "#f4f6fa" : "#202631",
          fillerColor: page.value.theme === "light" ? "#4777c420" : "#83a9ee20",
          handleStyle: {
            color: page.value.theme === "light" ? "#fff" : "#566379",
            borderColor: page.value.theme === "light" ? "#b9c7db" : "#75859f",
          },
          dataBackground: {
            lineStyle: { color: "#8899b3" },
            areaStyle: {
              color: page.value.theme === "light" ? "#d4ddec" : "#3b4659",
            },
          },
          selectedDataBackground: {
            lineStyle: { color: "#8da7cf" },
            areaStyle: {
              color: page.value.theme === "light" ? "#bdcdea" : "#4c5e7d",
            },
          },
          height: 16,
          bottom: 6,
          start: panel.value.zoom_start ?? 0,
          end: panel.value.zoom_end ?? 100,
        },
      ],
      series: series.map((s: Data, i: number) => ({
        name: s.name,
        type: "line",
        showSymbol: false,
        connectNulls: false,
        lineStyle: {
          width: panel.value.series?.[i]?.width || 2,
          color: panel.value.series?.[i]?.color,
        },
        itemStyle: { color: panel.value.series?.[i]?.color },
        data: s.points.map((p: Data) => [Number(p.time), p.value]),
      })),
    },
    { notMerge: true },
  );
  updating = false;
  chart.resize();
}
async function history(method: string) {
  if (!active()) return;
  working.value = true;
  try {
    await textCommit;
    if (!active()) return;
    if (filterText.value !== panel.value.text)
      await changeText(filterText.value);
    if (!active()) return;
    let args: Data = {
      streams: panel.value.streams,
      channels: panel.value.channels,
      text: panel.value.text,
      regex: panel.value.regex,
      time_from: panel.value.time_from,
      time_end: panel.value.time_end,
    };
    if (panel.value.kind === "curve") {
      const series = panel.value.series?.[0];
      if (!series) {
        ElMessage.warning("先添加至少一条曲线");
        return;
      }
      args = { ...args, page: pageId, panel: panelId };
      method = "history.curve";
    }
    const data = await ctx.command(method, args);
    if (!active()) return;
    await set({ mode: "history", query: data.query, offset: 0, paused: false });
    await update();
  } catch {
    // The shared command handler reports validation and revision errors.
  } finally {
    if (active()) working.value = false;
  }
}
async function context(event: any) {
  if (!active()) return;
  const row = event.data;
  const query = await ctx.command("history.context", {
    streams: [row.stream],
    epoch: row.epoch,
    seq: row.seq,
    byte_offset: row.offset,
    before: 10,
    after: 10,
  });
  if (!active()) return;
  await set({ mode: "history", query: query.query, offset: 0, paused: false });
  await update();
}
async function live() {
  await set({ mode: "live", query: null, offset: 0, paused: false });
  await update();
}
async function paginate(offset: number) {
  await set({ offset });
  await update();
}
const settingsPatch = computed(() => patchFrom(settingsBase.value, form.value, settingsFields));
const settingsConflicts = computed(() => panel.value.id ? conflictingFields(settingsBase.value, settingsPatch.value, normalizeSettings(panel.value)) : []);
function edit() {
  settingsBase.value = normalizeSettings(panel.value);
  form.value = clone(settingsBase.value);
  settingsError.value = "";
  settings.value = true;
}
function resolveSetting(key: string, mine: boolean) {
  const current = normalizeSettings(panel.value)[key];
  settingsBase.value[key] = clone(current);
  if (!mine) form.value[key] = clone(current);
  settingsError.value = "";
}
async function save() {
  if (!active()) return;
  const original = clone(settingsBase.value), changes = clone(settingsPatch.value);
  const wire = { ...changes };
  if (wire.streams) wire.streams = wire.streams.map((s: string) => JSON.parse(s));
  settingsSaving.value = true;
  try {
    await ctx.command("panel.set", { page: pageId, panel: props.id, ...wire }, {
      localError: true, editKey: `settings:${pageId}:${props.id}`, retryConflict: true,
      guard: (state) => {
        const p = state.pages.find((p: Data) => p.id === pageId)?.panels.find((p: Data) => p.id === props.id);
        return !!p && !conflictingFields(original, changes, normalizeSettings(p)).length;
      },
    });
    if (!active()) return;
    settings.value = false;
    await update();
  } catch (e) {
    if (!active()) return;
    settingsError.value = e instanceof Error ? e.message : String(e);
  } finally { settingsSaving.value = false; }
}
async function askRemove() {
  if (!active()) return;
  removeError.value = "";
  removeOpen.value = true;
  await nextTick();
  if (active()) cancelRemove.value?.focus();
}
function dismissRemove(event?: Event) {
  if (removing.value) return;
  if (event?.type === "pointerdown" && ((event.target as HTMLElement).closest('.panel-remove-confirm') || menuButton.value?.contains(event.target as Node))) return;
  removeOpen.value = false;
}
function removeKey(event: KeyboardEvent) { if (event.key === "Escape" && removeOpen.value) { dismissRemove(); menuButton.value?.focus(); } }
onMounted(() => { document.addEventListener('pointerdown', dismissRemove); document.addEventListener('keydown', removeKey); });
onBeforeUnmount(() => { document.removeEventListener('pointerdown', dismissRemove); document.removeEventListener('keydown', removeKey); });
async function remove() {
  if (!active()) return;
  removing.value = true;
  try {
    await ctx.command("panel.remove", { page: pageId, panel: panelId }, { localError: true, editKey: `remove:${props.id}` });
    ctx.panelDrafts.value.delete(`${pageId}:${panelId}`);
    ctx.clearEditError(`panel:${pageId}:${panelId}`);
    removeOpen.value = false;
  } catch (e) { removeError.value = e instanceof Error ? e.message : String(e); }
  finally { removing.value = false; }
}
async function duplicate() {
  if (!active()) return;
  const rect = place({ ...panel.value, id: "copy" } as any, page.value.panels, { x: 24, y: 24 },
    Math.max(320, (document.querySelector('.canvas-board')?.clientWidth ?? 1400) / page.value.view_zoom - 48));
  await ctx.command('panel.clone', { page: pageId, panel: panelId, left: rect.left, top: rect.top,
    y: Math.max(0, ...page.value.panels.map((p: Data) => p.y + p.h)) }).catch(() => {});
}
function editSeries(series?: Data) {
  seriesForm.value = series
    ? { ...series, streams: series.streams.map((s: Data) => bindingValue(s)) }
    : {
        name: "value",
        streams: [],
        regex: "(?P<value>-?[0-9]+(?:\\.[0-9]+)?)",
        field: "",
        color: "#50c8b8",
        width: 2,
      };
  seriesDialog.value = true;
  seriesForm.value.revision = ctx.state.value.revision;
}
async function saveSeries() {
  if (!active()) return;
  try {
    const value = {
      ...seriesForm.value,
      streams: seriesForm.value.streams.map((s: string) => JSON.parse(s)),
      page: pageId,
      panel: panelId,
      series: seriesForm.value.id,
    };
    await ctx.command(value.series ? "series.set" : "series.add", value);
    if (!active()) return;
    seriesDialog.value = false;
    await update();
  } catch {
    /* command already displays the error; keep the form open. */
  }
}
async function removeSeries(id: string) {
  if (!active()) return;
  await ctx.command("series.remove", {
    page: pageId,
    panel: panelId,
    series: id,
  });
}
function ready(event: any) {
  if (!active()) return;
  gridApi = event.api;
  if (panel.value.column_state)
    gridApi?.applyColumnState({
      state: panel.value.column_state,
      applyOrder: true,
    });
}
async function saveColumns(event: any) {
  if (
    !active() ||
    event.finished === false ||
    ![
      "uiColumnMoved",
      "uiColumnResized",
      "uiColumnSorted",
      "uiColumnDragged",
      "toolPanelDragAndDrop",
      "toolPanelUi",
      "contextMenu",
      "columnMenu",
    ].includes(event.source)
  )
    return;
  if (columnTimer) clearTimeout(columnTimer);
  const revision = ctx.state.value.revision;
  columnTimer = setTimeout(() => {
    if (!active()) return;
    const columnState = gridApi?.getColumnState();
    if (columnState)
      void set({
        revision,
        column_state: columnState,
        columns: columnState
          .filter((c: ColumnState) => !c.hide)
          .map((c: ColumnState) => c.colId),
      }).catch(() => {});
  }, 250);
}
onMounted(() => {
  // Register cleanup-owned resources before starting any asynchronous work.
  timer = setInterval(update, 750);
  observer = new ResizeObserver(() => chart?.resize());
  if (chartElement.value) observer.observe(chartElement.value);
  void update();
});
watch(
  () => page.value?.theme,
  () => {
    if (chart) draw(result.value);
  },
);
watch(
  () => JSON.stringify(panel.value.column_state),
  (next, old) => {
    if (next !== old && panel.value.column_state)
      gridApi?.applyColumnState({
        state: panel.value.column_state,
        applyOrder: true,
      });
  },
);
watch(
  () => JSON.stringify(panel.value),
  async () => {
    await update();
  },
);
onBeforeUnmount(() => {
  disposed = true;
  if (timer) clearInterval(timer);
  if (zoomTimer) clearTimeout(zoomTimer);
  if (columnTimer) clearTimeout(columnTimer);
  observer?.disconnect();
  chart?.dispose();
  chart = undefined;
  gridApi = undefined;
});
</script>
<template>
  <section
    class="panel nowheel"
    :class="{
      selected: page.active_panel === id,
      'panel-paused': panel.paused,
      nopan: page.tool !== 'pan',
      'pending-remove': removeOpen,
    }"
    :data-panel-id="id"
    v-if="panel.id"
  >
    <div class="panel-header">
      <div class="panel-drag" @click="ctx.selectPanel(id)">
        <Icon :name="panel.kind" /><strong>{{ panel.title }}</strong>
        <Icon v-if="panel.locked" name="lock" :size="12" />
      </div>
      <span
        class="panel-mode"
        :class="{ live: panel.mode === 'live' && !panel.paused }"
        >{{
          panel.paused ? "已暂停" : panel.mode === "history" ? "历史" : "实时"
        }}</span
      >
      <div class="panel-actions nodrag">
        <button
          class="icon-button"
          :aria-label="panel.paused ? '继续显示' : '暂停显示'"
          :title="panel.paused ? '继续显示' : '暂停显示（采集继续）'"
          @click.stop="set({ paused: !panel.paused })"
        >
          <Icon :name="panel.paused ? 'play' : 'pause'" />
        </button>
        <button
          class="icon-button"
          aria-label="面板属性"
          title="面板属性"
          @click.stop="ctx.selectPanel(id, true)"
        >
          <Icon name="inspector" />
        </button>
        <el-dropdown trigger="click" placement="bottom-end">
          <button ref="menuButton" class="icon-button" aria-label="面板菜单">
            <Icon name="more" />
          </button>
          <template #dropdown
            ><el-dropdown-menu>
              <el-dropdown-item @click="edit">筛选与显示设置</el-dropdown-item>
              <el-dropdown-item
                @click="duplicate"
                >复制面板</el-dropdown-item
              >
              <el-dropdown-item @click="set({ locked: !panel.locked })">{{
                panel.locked ? "解锁位置" : "锁定位置"
              }}</el-dropdown-item>
              <el-dropdown-item @click="set({ hidden: true })"
                >隐藏面板</el-dropdown-item
              >
              <el-dropdown-item divided class="danger-text" @click="askRemove"
                >删除面板…</el-dropdown-item
              >
            </el-dropdown-menu></template
          >
        </el-dropdown>
        <el-popover :visible="removeOpen" :virtual-ref="menuButton" virtual-triggering placement="bottom-end" :width="280" popper-class="panel-remove-confirm" :teleported="true">
          <div role="dialog" :aria-label="`删除面板 ${panel.title}`" @keydown.esc="dismissRemove()">
            <strong>删除“{{ panel.title }}”？</strong>
            <p>仅移除面板配置，日志数据保留。</p>
            <p v-if="removeError" class="danger-text" role="alert">{{ removeError }}</p>
            <div class="confirm-actions">
              <button ref="cancelRemove" class="text-button" :disabled="removing" @click="dismissRemove(); menuButton?.focus()">取消</button>
              <el-button size="small" type="danger" :loading="removing" @click="remove">删除面板</el-button>
            </div>
          </div>
        </el-popover>
      </div>
    </div>
    <div class="panel-toolbar nodrag">
      <el-input
        v-model="filterText"
        placeholder="筛选日志…"
        clearable
        size="small"
        aria-label="筛选日志"
        @focus="filterRevision = ctx.state.value.revision"
        @change="changeText"
        ><template #prefix><Icon name="search" :size="14" /></template
      ></el-input>
      <button
        class="text-button"
        :class="{ active: panel.mode === 'live' }"
        @click="live"
      >
        实时
      </button>
      <button
        class="text-button"
        :class="{ active: panel.mode === 'history' }"
        :disabled="working"
        @click="history('history.read')"
      >
        历史
      </button>
      <el-dropdown trigger="click" placement="bottom-end"
        ><button class="icon-button" aria-label="查询操作" title="查询操作">
          <Icon name="search" :size="14" />
        </button>
        <template #dropdown
          ><el-dropdown-menu
            ><el-dropdown-item @click="history('history.search')"
              >搜索全部可用上下文</el-dropdown-item
            ><el-dropdown-item @click="edit"
              >设置正则与时间范围</el-dropdown-item
            ></el-dropdown-menu
          ></template
        >
      </el-dropdown>
    </div>
    <div
      v-if="result.error || result.status?.state === 'failed'"
      class="inline-error"
    >
      <Icon name="warning" />{{ result.error || result.status?.error }}
    </div>
    <div v-if="waiting" class="waiting">
      <Icon name="stream" />等待来源连接，配置已保留
    </div>
    <AgGridVue
      v-if="panel.kind === 'log'"
      class="log-grid nodrag"
      :theme="theme"
      :row-data="rows"
      :column-defs="columns"
      :default-col-def="{ resizable: true }"
      :row-height="panel.row_height || 28"
      :locale-text="{
        noRowsToShow: filterText ? '没有匹配的日志' : '暂无日志，等待来源数据',
      }"
      :animate-rows="false"
      :suppress-cell-focus="false"
      :row-class-rules="{
        'log-error': (p: any) =>
          p.data?.channel === 'stderr' ||
          /\b(ERROR|FATAL)\b/i.test(p.data?.text || ''),
        'log-warning': (p: any) => /\bWARN(?:ING)?\b/i.test(p.data?.text || ''),
      }"
      :get-row-id="
        (p) =>
          [
            p.data.stream,
            p.data.epoch,
            p.data.seq,
            p.data.offset,
            p.data.kind,
          ].join(':')
      "
      @grid-ready="ready"
      @row-double-clicked="context"
      @column-resized="saveColumns"
      @column-moved="saveColumns"
      @sort-changed="saveColumns"
    />
    <div v-else class="chart-wrapper nodrag">
      <div v-if="!panel.series.length" class="chart-empty">
        <Icon name="curve" :size="28" /><strong>添加第一条曲线</strong
        ><span>从日志中的数值或 JSON 字段开始</span
        ><el-button size="small" @click="editSeries()">添加曲线</el-button>
      </div>
      <div v-show="panel.series.length" ref="chartElement" class="chart"></div>
      <div class="series-chips">
        <button
          v-for="s in panel.series"
          :key="s.id"
          class="series-chip"
          @click="editSeries(s)"
        >
          <i :style="{ background: s.color }"></i>{{ s.name }}</button
        ><button class="text-button" title="添加曲线" @click="editSeries()">
          <Icon name="plus" :size="12" />
        </button>
      </div>
    </div>
    <div class="panel-footer nodrag">
      <el-popover placement="top-start" width="360" trigger="click">
        <template #reference
          ><button
            class="coverage-trigger"
            :class="{ warning: coverageStatus.tone === 'warning' }"
            :title="sourceLabel"
          >
            <span
              class="status-dot"
              :class="{
                warning: coverageStatus.tone === 'warning',
                offline: coverageStatus.tone === 'neutral',
              }"
            ></span
            >{{ coverageStatus.label }}<span class="footer-source">
              · {{ sourceLabel }}</span
            >
          </button></template
        >
        <div class="coverage-details">
          <p>{{ coverageStatus.description }}</p>
          <em v-if="coverageStatus.error">{{ coverageStatus.error }}</em>
          <template v-if="coverage">
            <strong>{{
              coverage.mode === "memory_only" ? "当前内存覆盖" : "归档与内存覆盖"
            }}</strong>
            <p v-for="s in coverage.streams" :key="s.stream">
              归档
              {{
                s.archived
                  ? s.archived.empty
                    ? "等待提交"
                    : `${s.archived.first}–${s.archived.last}`
                  : "不可用"
              }}
              · 内存 {{ s.memory.first || "无" }}–{{ s.memory.last || "无"
              }}<em
                v-if="
                  s.uncovered_prefix ||
                  s.uncovered_between ||
                  s.uncovered_after ||
                  s.memory.range_count > 1
                "
                >存在未覆盖区间</em
              ><em v-if="s.uncommitted && coverage.mode !== 'memory_only'"
                >尚未提交</em
              >
            </p>
            <em v-if="coverage.runtime_match === false">归档未匹配本次运行</em
            ><em v-if="coverage.writer?.report?.state === 'failed'"
              >归档写入失败：{{ coverage.writer.report.error }}</em
            ><em v-if="coverage.archive_error"
              >归档故障：{{ coverage.archive_error }}</em
            ><em v-if="coverage.gap_count">{{ coverage.gap_count }} 个归档缺口</em
            ><em v-if="coverage.writer?.connected === false"
              >归档已停止，保留已提交前缀</em
            >
          </template>
        </div>
      </el-popover>
      <span v-if="queryRunning" class="scan-progress"
        >扫描 {{ result.status.scanned }} 条</span
      >
      <span v-else-if="panel.kind === 'log'" class="row-count"
        >{{ rows.length }} 行</span
      >
      <template v-if="panel.mode === 'history' && panel.kind === 'log'"
        ><button
          class="icon-button compact"
          :disabled="!panel.offset"
          title="上一页"
          aria-label="上一页"
          @click="paginate(Math.max(0, (panel.offset || 0) - 200))"
        >
          <Icon name="back" :size="12" /></button
        ><span>{{ panel.offset || 0 }} / {{ result.total || 0 }}</span
        ><button
          class="icon-button compact"
          :disabled="(result.next || 0) >= (result.total || 0)"
          title="下一页"
          aria-label="下一页"
          @click="paginate(result.next)"
        >
          <Icon name="next" :size="12" /></button
      ></template>
      <button
        v-if="queryRunning"
        class="text-button"
        @click="ctx.command('query.cancel', { query: panel.query })"
      >
        取消
      </button>
      <button
        v-if="panel.kind === 'log' && panel.mode === 'live'"
        class="follow-button"
        :class="{ active: panel.follow }"
        :aria-pressed="panel.follow"
        title="自动跟随最新记录"
        @click="set({ follow: !panel.follow })"
      >
        跟随
      </button>
    </div>
  </section>
  <el-dialog v-model="settings" title="筛选与显示" width="580" append-to-body>
    <el-form label-position="top" class="settings-form" :disabled="settingsSaving">
      <el-form-item label="标题">
        <el-input v-model="form.title" />
      </el-form-item>
      <el-form-item label="来源（留空表示全部流）">
        <el-select v-model="form.streams" multiple filterable>
          <el-option
            v-for="s in streamOptions"
            :key="s.value"
            :value="s.value"
            :label="s.label"
          />
        </el-select>
      </el-form-item>
      <el-form-item label="通道">
        <el-select v-model="form.channels" multiple allow-create filterable>
          <el-option value="stdout" label="stdout" />
          <el-option value="stderr" label="stderr" />
        </el-select>
      </el-form-item>
      <el-form-item label="正则过滤">
        <el-input v-model="form.regex" placeholder="Rust regex" />
      </el-form-item>
      <el-form-item label="显示">
        <el-radio-group v-model="form.format">
          <el-radio-button value="text">文本</el-radio-button>
          <el-radio-button value="hex">十六进制</el-radio-button>
        </el-radio-group>
        <el-switch v-model="form.follow" active-text="自动跟随" />
        <el-switch v-model="form.metadata" active-text="元数据" />
      </el-form-item>
      <el-form-item label="可见列">
        <el-select v-model="form.columns" multiple>
          <el-option v-for="c in allColumns" :key="c" :value="c" :label="c" />
        </el-select>
      </el-form-item>
      <template v-if="form.kind === 'curve'">
        <el-form-item label="图例">
          <el-switch v-model="form.legend" />
        </el-form-item>
        <el-form-item label="Y 范围">
          <el-input-number v-model="form.y_min" placeholder="自动" />
          <el-input-number v-model="form.y_max" placeholder="自动" />
        </el-form-item>
      </template>
      <el-form-item label="观测时间范围（Unix 纳秒，可留空）">
        <el-input v-model="form.time_from" placeholder="起点纳秒" />
        <el-input v-model="form.time_end" placeholder="终点纳秒" />
      </el-form-item>
    </el-form>
    <template #footer>
      <div v-if="settingsError || settingsConflicts.length" class="draft-feedback" role="alert">
        <p>{{ settingsError }}</p>
        <div v-for="key in settingsConflicts" :key="key" class="field-conflict">
          <strong>{{ fieldLabels[key] || key }}已被修改</strong>
          <span>当前：{{ normalizeSettings(panel)[key] }} · 我的：{{ form[key] }}</span>
          <el-button @click="resolveSetting(key, false)">采用当前值</el-button><el-button @click="resolveSetting(key, true)">保留我的修改</el-button>
        </div>
      </div>
      <el-button type="primary" :loading="settingsSaving" :disabled="settingsConflicts.length > 0" @click="save">保存设置</el-button>
    </template>
  </el-dialog>
  <el-dialog v-model="seriesDialog" title="曲线定义" width="480" append-to-body>
    <el-form label-position="top">
      <el-form-item label="名称">
        <el-input v-model="seriesForm.name" />
      </el-form-item>
      <el-form-item label="来源">
        <el-select v-model="seriesForm.streams" multiple>
          <el-option
            v-for="s in streamOptions"
            :key="s.value"
            :value="s.value"
            :label="s.label"
          />
        </el-select>
      </el-form-item>
      <el-form-item label="正则（命名捕获组 value）">
        <el-input v-model="seriesForm.regex" />
      </el-form-item>
      <el-form-item label="或 JSON 字段路径">
        <el-input
          v-model="seriesForm.field"
          placeholder="metrics.temperature"
        />
      </el-form-item>
      <el-form-item label="颜色 / 线宽">
        <el-color-picker v-model="seriesForm.color" />
        <el-input-number
          v-model="seriesForm.width"
          :min="0.1"
          :max="20"
          :step="0.5"
        />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button
        v-if="seriesForm.id"
        type="danger"
        plain
        @click="removeSeries(seriesForm.id).then(() => (seriesDialog = false))"
        >删除曲线</el-button
      ><el-button type="primary" @click="saveSeries">保存曲线</el-button>
    </template>
  </el-dialog>
</template>
