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
import { useGridStack, useGridStackItem } from "gridstack/dist/vue";
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
import { ElMessage, ElMessageBox } from "element-plus";
import { AppContext, type Data } from "./api";
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
const gridStack = useGridStack(),
  gridItem = useGridStackItem();
const page = computed(() =>
  ctx.state.value.pages.find((p: Data) => p.id === ctx.state.value.selected),
);
const panel = computed<Data>(
  () => page.value?.panels.find((p: Data) => p.id === props.id) || {},
);
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
    backgroundColor: page.value?.theme === "light" ? "#fff" : "#151d27",
    foregroundColor: page.value?.theme === "light" ? "#253348" : "#dce6f2",
    borderColor: page.value?.theme === "light" ? "#dde3eb" : "#293444",
    fontSize: 12,
    headerBackgroundColor:
      page.value?.theme === "light" ? "#f4f6f9" : "#1a2531",
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
              })
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
    value: JSON.stringify(
      s.alias
        ? { owner: s.owner, alias: s.alias }
        : { owner: s.owner, alias: null, stream: s.id, epoch: s.epoch },
    ),
    label: s.alias || `${s.owner} / ${s.id.slice(0, 8)}`,
  })),
);
const coverage = computed(() => result.value.status?.coverage);
const waiting = computed(
  () => result.value.waiting === true || result.value.status?.waiting === true,
);
async function set(args: Data) {
  await ctx.command("panel.set", {
    page: page.value.id,
    panel: props.id,
    ...args,
  });
}
async function update() {
  if (fetching) return;
  fetching = true;
  try {
    const p = panel.value;
    let data: Data;
    if (p.paused) {
      data = await ctx.command("panel.data", {
        page: page.value.id,
        panel: props.id,
      });
      rows.value = data.rows || [];
    } else if (p.mode === "history" && p.query) {
      data = await ctx.command("query.get", {
        query: p.query,
        offset: p.offset || 0,
      });
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
        page: page.value.id,
        panel: props.id,
      });
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
      const count = gridApi?.getDisplayedRowCount() || 0;
      if (count) gridApi?.ensureIndexVisible(count - 1, "bottom");
    }
  } catch (e) {
    result.value = { error: String(e) };
  } finally {
    fetching = false;
  }
}
function draw(data: Data) {
  if (!chartElement.value) return;
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
      textStyle: { color: "#899bb1" },
      tooltip: { trigger: "axis" },
      legend: {
        show: panel.value.legend,
        selected: panel.value.legend_selected || {},
        top: 4,
        textStyle: { color: "#a8b8cc" },
      },
      grid: { left: 55, right: 24, top: 40, bottom: 62 },
      xAxis: { type: "time", axisLine: { lineStyle: { color: "#344458" } } },
      yAxis: {
        type: "value",
        min: panel.value.y_min ?? null,
        max: panel.value.y_max ?? null,
        splitLine: { lineStyle: { color: "#253242" } },
      },
      dataZoom: [
        {
          type: "inside",
          start: panel.value.zoom_start ?? 0,
          end: panel.value.zoom_end ?? 100,
        },
        {
          type: "slider",
          height: 18,
          bottom: 12,
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
  working.value = true;
  try {
    await textCommit;
    if (filterText.value !== panel.value.text)
      await changeText(filterText.value);
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
      args = { ...args, page: page.value.id, panel: props.id };
      method = "history.curve";
    }
    const data = await ctx.command(method, args);
    await set({ mode: "history", query: data.query, offset: 0, paused: false });
    await update();
  } catch {
    // The shared command handler reports validation and revision errors.
  } finally {
    working.value = false;
  }
}
async function context(event: any) {
  const row = event.data;
  const query = await ctx.command("history.context", {
    streams: [row.stream],
    epoch: row.epoch,
    seq: row.seq,
    byte_offset: row.offset,
    before: 10,
    after: 10,
  });
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
function edit() {
  form.value = {
    ...panel.value,
    revision: ctx.state.value.revision,
    streams: panel.value.streams.map((s: Data) => JSON.stringify(s)),
  };
  settings.value = true;
}
async function save() {
  try {
    const value: Data = {
      ...form.value,
      streams: form.value.streams.map((s: string) => JSON.parse(s)),
    };
    delete value.id;
    delete value.series;
    await set(value);
    settings.value = false;
    await update();
  } catch {
    /* command already displays the error; keep the form open. */
  }
}
async function remove() {
  try {
    await ElMessageBox.confirm("删除这个面板？", "移除面板");
    await ctx.command("panel.remove", { page: page.value.id, panel: props.id });
  } catch (e) {
    if (!["cancel", "close"].includes(String(e))) throw e;
  }
}
function editSeries(series?: Data) {
  seriesForm.value = series
    ? { ...series, streams: series.streams.map((s: Data) => JSON.stringify(s)) }
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
  try {
    const value = {
      ...seriesForm.value,
      streams: seriesForm.value.streams.map((s: string) => JSON.parse(s)),
      page: page.value.id,
      panel: props.id,
      series: seriesForm.value.id,
    };
    await ctx.command(value.series ? "series.set" : "series.add", value);
    seriesDialog.value = false;
    await update();
  } catch {
    /* command already displays the error; keep the form open. */
  }
}
async function removeSeries(id: string) {
  await ctx.command("series.remove", {
    page: page.value.id,
    panel: props.id,
    series: id,
  });
}
function ready(event: any) {
  gridApi = event.api;
  if (panel.value.column_state)
    gridApi?.applyColumnState({
      state: panel.value.column_state,
      applyOrder: true,
    });
}
async function saveColumns(event: any) {
  if (
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
onMounted(async () => {
  await nextTick();
  if (gridItem.node?.el) gridStack.grid?.refreshDragHandles(gridItem.node.el);
  await update();
  timer = setInterval(update, 750);
  observer = new ResizeObserver(() => chart?.resize());
  if (chartElement.value) observer.observe(chartElement.value);
});
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
  if (timer) clearInterval(timer);
  if (zoomTimer) clearTimeout(zoomTimer);
  if (columnTimer) clearTimeout(columnTimer);
  observer?.disconnect();
  chart?.dispose();
});
</script>
<template>
  <section class="panel" v-if="panel.id">
    <div class="panel-header">
      <div class="panel-drag">
        <span>{{ panel.kind === "log" ? "▤" : "⌁" }}</span>
        <strong>{{ panel.title }}</strong>
        <small>{{ panel.mode === "history" ? "固定历史" : "实时" }}</small>
      </div>
      <div>
        <el-button text size="small" @click="set({ paused: !panel.paused })">
          {{ panel.paused ? "▶ 继续" : "Ⅱ 暂停" }}
        </el-button>
        <el-button text size="small" @click="edit">设置</el-button>
        <el-button text size="small" @click="remove">×</el-button>
      </div>
    </div>
    <div class="panel-toolbar">
      <el-input
        v-model="filterText"
        placeholder="筛选内容…"
        clearable
        size="small"
        @focus="filterRevision = ctx.state.value.revision"
        @change="changeText"
      />
      <el-button
        size="small"
        :loading="working"
        @click="history('history.read')"
      >
        历史
      </el-button>
      <el-button size="small" @click="history('history.search')">
        全范围搜索
      </el-button>
      <el-button
        size="small"
        :type="panel.mode === 'live' ? 'primary' : 'default'"
        @click="live"
      >
        实时
      </el-button>
      <el-button
        v-if="panel.kind === 'curve'"
        size="small"
        @click="editSeries()"
      >
        ＋ 曲线
      </el-button>
    </div>
    <div v-if="coverage" class="coverage">
      <b>
        {{ coverage.mode === "memory_only" ? "仅内存范围" : "归档 + 内存" }}
      </b>
      <span v-for="s in coverage.streams" :key="s.stream">
        归档
        {{
          s.archived
            ? s.archived.empty
              ? "等待提交"
              : `${s.archived.first}–${s.archived.last}`
            : "不可用"
        }}
        · 内存 {{ s.memory.first || "无" }}–{{ s.memory.last || "无" }}
        <em
          v-if="
            s.uncovered_prefix ||
            s.uncovered_between ||
            s.uncovered_after ||
            s.memory.range_count > 1
          "
        >
          存在未覆盖区间
        </em>
        <em v-if="s.uncommitted && coverage.mode !== 'memory_only'">
          尚未提交
        </em>
      </span>
      <em v-if="coverage.runtime_match === false">归档未匹配本次运行</em>
      <em v-if="coverage.writer?.report?.state === 'failed'">
        归档写入失败：{{ coverage.writer.report.error }}
      </em>
      <em v-if="coverage.archive_error">
        归档故障：{{ coverage.archive_error }}
      </em>
      <em v-if="coverage.gap_count">{{ coverage.gap_count }} 个归档缺口</em>
      <em v-if="coverage.writer && coverage.writer.connected === false">
        归档已停止，保留已提交前缀
      </em>
    </div>
    <el-alert
      v-if="result.error || result.status?.state === 'failed'"
      :title="result.error || result.status?.error"
      type="warning"
      :closable="false"
    />
    <div v-if="waiting" class="waiting">等待来源 · 保留面板配置</div>
    <AgGridVue
      v-if="panel.kind === 'log'"
      class="log-grid"
      :theme="theme"
      :row-data="rows"
      :column-defs="columns"
      :default-col-def="{ resizable: true }"
      :row-height="30"
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
    <div v-else class="chart-wrapper">
      <div class="series-chips">
        <el-tag
          v-for="s in panel.series"
          :key="s.id"
          closable
          @click="editSeries(s)"
          @close="removeSeries(s.id)"
        >
          {{ s.name }}
        </el-tag>
        <span v-if="!panel.series.length">添加正则或 JSON 数值曲线</span>
      </div>
      <div ref="chartElement" class="chart"></div>
    </div>
    <div class="panel-footer">
      <span>
        {{
          panel.paused
            ? "显示已暂停，仍在采集"
            : result.status?.state === "running"
              ? `扫描中 · ${result.status.scanned} Records`
              : panel.kind === "log"
                ? `${rows.length} 行 · 双击定位上下文`
                : "拖动滑块或滚轮缩放"
        }}
      </span>
      <template v-if="panel.mode === 'history' && panel.kind === 'log'">
        <el-button
          text
          size="small"
          :disabled="!panel.offset"
          @click="paginate(Math.max(0, (panel.offset || 0) - 200))"
        >
          上一页
        </el-button>
        <span>{{ panel.offset || 0 }} / {{ result.total || 0 }}</span>
        <el-button
          text
          size="small"
          :disabled="(result.next || 0) >= (result.total || 0)"
          @click="paginate(result.next)"
        >
          下一页
        </el-button>
      </template>
      <el-button
        v-if="panel.mode === 'history' && result.status?.state === 'running'"
        text
        size="small"
        @click="ctx.command('query.cancel', { query: panel.query })"
      >
        取消查询
      </el-button>
    </div>
  </section>
  <el-dialog v-model="settings" title="面板设置" width="660">
    <el-form label-position="top" class="settings-form">
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
      <el-button type="primary" @click="save">保存设置</el-button>
    </template>
  </el-dialog>
  <el-dialog v-model="seriesDialog" title="曲线定义" width="540">
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
      <el-button type="primary" @click="saveSeries">保存曲线</el-button>
    </template>
  </el-dialog>
</template>
