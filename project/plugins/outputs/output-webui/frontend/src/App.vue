<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, provide, ref, watch } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { AppContext, call, RevisionConflict, type CommandOptions, type Data } from "./api";
import CanvasBoard from "./CanvasBoard.vue";
import GridBoard from "./GridBoard.vue";
import Inspector from "./Inspector.vue";
import Icon from "./Icon.vue";
import type { PanelDraft } from "./panelDraft";
import { place } from "./layout";
const state = ref<Data>({ pages: [], streams: [], revision: 0 });
const online = ref(false),
  pending = ref(0),
  loadError = ref(""),
  search = ref(""),
  settings = ref(false),
  cliHelp = ref(false),
  pageForm = ref<Data>({});
const deferredEdits = ref(0);
const panelDrafts = ref(new Map<string, PanelDraft>());
const failedEdits = ref(new Map<string, string>());
const saving = computed(() => pending.value > 0 || deferredEdits.value > 0);
const saveError = computed(() => [...failedEdits.value.values()].join("\n"));
const canvas = ref<InstanceType<typeof CanvasBoard>>();
const page = computed<Data>(() =>
  state.value.pages.find((p: Data) => p.id === state.value.selected),
);
const pages = computed(() =>
  [...state.value.pages].sort((a: Data, b: Data) => a.order - b.order),
);
const streams = computed(() =>
  state.value.streams.filter((s: Data) =>
    `${s.alias} ${s.owner} ${s.description}`
      .toLowerCase()
      .includes(search.value.toLowerCase()),
  ),
);
const visiblePanels = computed(
  () => page.value?.panels.filter((p: Data) => !p.hidden) || [],
);
const archiveIssue = computed(
  () =>
    state.value.archive_writer?.report?.state === "failed" ||
    (state.value.archive_enabled &&
      state.value.archive_writer?.connected === false),
);
function apply(next: Data) {
  if (next.revision >= state.value.revision)
    state.value = { ...state.value, ...next };
}
async function refresh() {
  const response = await fetch("/api/state");
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  apply(await response.json());
  loadError.value = "";
}
let editQueue: Promise<unknown> = Promise.resolve();
const readMethods = new Set([
  "panel.data",
  "query.get",
  "history.read",
  "history.search",
  "history.context",
  "history.curve",
  "query.cancel",
  "page.get",
  "page.list",
  "panel.get",
  "streams",
  "capabilities",
  "url",
]);
async function command(method: string, args: Data = {}, options: CommandOptions = {}) {
  if (readMethods.has(method)) {
    try {
      return await call(method, args);
    } catch (e) {
      if (method.startsWith("history.") || method === "query.cancel")
        ElMessage.error(e instanceof Error ? e.message : String(e));
      throw e;
    }
  }
  // A later unrelated success must not hide an earlier failed edit.
  const editKey = options.editKey ?? JSON.stringify([method, args.page, args.panel,
    Object.keys(args).filter((key) => key !== "revision").sort()]);
  pending.value++;
  const task = editQueue
    .catch(() => {})
    .then(async () => {
      try {
        let result: Data;
        for (let attempt = 0; ; attempt++) {
          if (options.guard && !options.guard(state.value)) throw new RevisionConflict();
          try {
            result = await call(method, { revision: state.value.revision, ...args });
            break;
          } catch (e) {
            if (!(e instanceof RevisionConflict) || !options.guard || !options.retryConflict || attempt >= 1) throw e;
            await refresh();
          }
        }
        if (result.state?.pages) apply(result.state);
        else await refresh();
        failedEdits.value.delete(editKey);
        return result;
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        failedEdits.value.set(editKey, message);
        if (!options.localError) ElMessage({ type: "error", message, grouping: true });
        await refresh().catch(() => {});
        throw e;
      } finally {
        pending.value--;
      }
    });
  editQueue = task;
  return task;
}
function run(method: string, args: Data = {}) {
  void command(method, args).catch(() => {});
}
async function selectPanel(id: string | null, inspect = false) {
  if (page.value.active_panel === id && (!inspect || page.value.inspector_open))
    return;
  await command("page.set", {
    page: page.value.id,
    active_panel: id,
    ...(inspect ? { inspector_open: true } : {}),
  }).catch(() => {});
}
provide(AppContext, { state, deferredEdits, panelDrafts, command, refresh, selectPanel,
  clearEditError: (key) => { failedEdits.value.delete(key); } });
watch(
  () => page.value?.theme,
  (theme) =>
    document.documentElement.classList.toggle("dark", theme === "dark"),
  { immediate: true },
);
let source: EventSource | undefined;
onMounted(async () => {
  try {
    await refresh();
  } catch (e) {
    loadError.value = String(e);
  }
  source = new EventSource("/api/events");
  source.addEventListener("state", (e) => {
    apply(JSON.parse((e as MessageEvent).data));
    online.value = true;
    loadError.value = "";
  });
  source.onerror = () => (online.value = false);
  document.addEventListener("keydown", keyboard);
});
onBeforeUnmount(() => {
  source?.close();
  document.removeEventListener("keydown", keyboard);
});
function pageSet(args: Data) {
  run("page.set", { page: page.value.id, ...args });
}
async function createPage() {
  try {
    const result = await ElMessageBox.prompt(
      "名称也可用于 CLI 定位这个工作台。",
      "新建工作台",
      {
        inputPlaceholder: "例如：设备调试",
        confirmButtonText: "创建",
        cancelButtonText: "取消",
        inputValidator: (v: string) => !!v.trim() || "请输入名称",
      },
    );
    const made = await command("page.create", {
      name: result.value.trim(),
      title: result.value.trim(),
    });
    await command("page.select", { page: made.result.id });
  } catch {}
}
async function clonePage() {
  try {
    const result = await ElMessageBox.prompt(
      "为副本输入一个唯一名称。",
      "复制工作台",
      {
        inputValue: `${page.value.name}-copy`,
        confirmButtonText: "复制",
        cancelButtonText: "取消",
      },
    );
    const made = await command("page.clone", {
      page: page.value.id,
      name: result.value,
      title: result.value,
    });
    await command("page.select", { page: made.result.id });
  } catch {}
}
async function removePage() {
  try {
    await ElMessageBox.confirm(
      `删除“${page.value.title}”及其中的面板配置？`,
      "删除工作台",
      { confirmButtonText: "删除", cancelButtonText: "取消", type: "warning" },
    );
    await command("page.delete", { page: page.value.id });
  } catch {}
}
function editPage() {
  pageForm.value = { ...page.value, revision: state.value.revision };
  settings.value = true;
}
async function savePage() {
  try {
    await command("page.set", {
      page: pageForm.value.id,
      name: pageForm.value.name,
      title: pageForm.value.title,
      theme: pageForm.value.theme,
      order: pageForm.value.order,
      revision: pageForm.value.revision,
    });
    settings.value = false;
  } catch {}
}
async function add(kind: string, stream?: Data, geometry: Data = {}) {
  const n = page.value.panels.length;
  const placement = place({ id: "new", left: 24, top: 24,
    panel_width: kind === "log" ? 760 : 600, panel_height: kind === "log" ? 440 : 360 },
    page.value.panels, { x: 24, y: 24 }, canvas.value?.availableWidth() ?? 1400);
  const made = await command("panel.add", {
    page: page.value.id,
    kind,
    title: stream
      ? `${stream.alias || stream.owner} · 日志`
      : kind === "log"
        ? "日志监视器"
        : "数值趋势",
    x: 0,
    y: Math.max(0, ...page.value.panels.map((p: Data) => p.y + p.h)),
    w: kind === "log" ? 8 : 6,
    h: 6,
    left: placement.left,
    top: placement.top,
    panel_width: kind === "log" ? 760 : 600,
    panel_height: kind === "log" ? 440 : 360,
    z_index: n,
    ...(stream
      ? {
          streams: [
            stream.alias
              ? { owner: stream.owner, alias: stream.alias }
              : {
                  owner: stream.owner,
                  stream: stream.id,
                  epoch: stream.epoch,
                  alias: null,
                },
          ],
        }
      : {}),
    ...geometry,
  });
  await selectPanel(made.result.id, true);
  return made.result;
}
async function starter() {
  try {
    await add("log", undefined, {
      left: 24,
      top: 24,
      panel_width: 760,
      panel_height: 540,
      w: 8,
      h: 8,
    });
    await add("curve", undefined, {
      left: 808,
      top: 24,
      panel_width: 520,
      panel_height: 340,
      x: 8,
      y: 0,
      w: 4,
      h: 5,
    });
    await command("page.set", {
      page: page.value.id,
      active_panel: null,
      inspector_open: false,
    });
    await canvas.value?.fit();
  } catch {}
}
function geometryReset() {
  void canvas.value?.reset();
}
function keyboard(e: KeyboardEvent) {
  if (
    !page.value ||
    settings.value ||
    cliHelp.value ||
    (e.target as HTMLElement)?.closest(
      'input,textarea,select,[contenteditable="true"],.el-dialog,.ag-root',
    )
  )
    return;
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  if (e.key === "Escape") {
    void selectPanel(null);
    return;
  }
  if (e.key === "v") {
    e.preventDefault();
    pageSet({ tool: "select" });
  }
  if (e.key === "h") {
    e.preventDefault();
    pageSet({ tool: "pan" });
  }
  if (e.key === "0") {
    e.preventDefault();
    void canvas.value?.fit();
  }
  if (e.key === "+" || e.key === "=") {
    e.preventDefault();
    void canvas.value?.zoom(1.2);
  }
  if (e.key === "-") {
    e.preventDefault();
    void canvas.value?.zoom(1 / 1.2);
  }
}
function shellQuote(value: string) {
  return "'" + value.replaceAll("'", "'\\''") + "'";
}
const cliExample = computed(
  () =>
    `log-print webui WEB page set --page ${shellQuote(page.value?.name || "overview")} --layout-mode canvas\nlog-print webui WEB panel add --kind log --title '串口日志' --left 24 --top 24 --panel-width 760 --panel-height 480\nlog-print webui WEB panel set --panel PANEL_ID --left 820 --top 24 --panel-width 560 --panel-height 360\nlog-print webui WEB page set --view-x 24 --view-y 24 --view-zoom 0.8\nlog-print webui WEB page set --inspector-open false --show-minimap true\nlog-print webui WEB layout set --place PANEL_ID=24,24,760,480`,
);
async function copyCli() {
  try {
    await navigator.clipboard.writeText(cliExample.value);
    ElMessage.success("已复制命令");
  } catch {
    ElMessage.info("请选中命令后复制");
  }
}
</script>
<template>
  <div
    class="workbench"
    :class="{
      'sidebar-hidden': !page?.sidebar_open,
      'inspector-visible': page?.inspector_open,
    }"
  >
    <header class="app-header">
      <div class="app-brand">
        <img class="brand-symbol" src="/log-print-icon.svg" alt="" />
        <strong>log_print</strong><span class="brand-divider"></span
        ><span class="app-caption">工作台</span>
      </div>
      <div class="connection-summary">
        <span class="status-dot" :class="{ offline: !online }"></span
        ><span>{{ online ? "已连接" : "重新连接中" }}</span
        ><span class="header-separator">/</span
        ><Icon name="stream" :size="14" /><span
          >{{ state.streams.length }} 个流</span
        ><span class="header-separator">/</span
        ><span :class="{ 'warning-text': archiveIssue }">{{
          archiveIssue
            ? "归档需关注"
            : state.archive_enabled
              ? "归档已启用"
              : "仅实时缓存"
        }}</span>
      </div>
      <div class="app-actions">
        <button class="text-button cli-trigger" @click="cliHelp = true">
          <span class="terminal-symbol">&gt;_</span>CLI 控制</button
        ><button
          class="icon-button"
          aria-label="切换主题"
          title="切换明暗主题"
          :disabled="!page"
          @click="pageSet({ theme: page.theme === 'dark' ? 'light' : 'dark' })"
        >
          <Icon name="view" />
        </button>
      </div>
    </header>
    <div v-if="loadError && !page" class="load-state">
      <Icon name="warning" :size="28" />
      <h2>暂时无法读取工作台</h2>
      <p>{{ loadError }}</p>
      <el-button @click="refresh">重试</el-button>
    </div>
    <template v-else-if="page">
      <aside
        v-if="page.sidebar_open"
        class="workspace-sidebar"
        aria-label="工作台导航"
      >
        <div class="rail-header">
          <span>工作台</span
          ><button
            class="icon-button"
            title="新建工作台"
            aria-label="新建工作台"
            @click="createPage"
          >
            <Icon name="plus" />
          </button>
        </div>
        <nav class="page-list">
          <button
            v-for="p in pages"
            :key="p.id"
            :class="['page-link', { active: p.id === state.selected }]"
            @click="run('page.select', { page: p.id })"
          >
            <Icon name="page" /><span>{{ p.title }}</span
            ><small>{{ p.panels.length }}</small>
          </button>
        </nav>
        <div class="sidebar-section-title">
          <span>面板</span><small>{{ page.panels.length }}</small>
        </div>
        <div class="layer-list">
          <div
            v-for="p in page.panels"
            :key="p.id"
            :class="[
              'layer-row',
              { active: p.id === page.active_panel, muted: p.hidden },
            ]"
          >
            <button
              class="layer-name"
              @click="selectPanel(p.id, true)"
              @dblclick="canvas?.fit(p.id)"
            >
              <Icon :name="p.kind" :size="14" /><span>{{ p.title }}</span
              ><Icon v-if="p.locked" name="lock" :size="11" /></button
            ><button
              class="icon-button compact layer-visibility"
              :title="p.hidden ? '显示面板' : '隐藏面板'"
              :aria-label="`${p.hidden ? '显示' : '隐藏'} ${p.title}`"
              @click="run('panel.set', { panel: p.id, hidden: !p.hidden })"
            >
              <Icon :name="p.hidden ? 'hide' : 'view'" :size="13" />
            </button>
          </div>
          <p v-if="!page.panels.length" class="sidebar-hint">
            添加面板后，在这里管理显示与层级。
          </p>
        </div>
        <div class="sources-section">
          <div class="sidebar-section-title">
            <span>数据来源</span><small>{{ state.streams.length }}</small>
          </div>
          <el-input
            v-model="search"
            placeholder="查找流"
            size="small"
            clearable
            aria-label="查找流"
            ><template #prefix><Icon name="search" :size="13" /></template
          ></el-input>
          <div class="source-list">
            <el-popover
              v-for="s in streams"
              :key="s.id"
              placement="right-start"
              :width="280"
              trigger="hover"
              :show-after="500"
              ><template #reference
                ><button class="source-row" @click="add('log', s)">
                  <span class="source-type"
                    ><Icon name="stream" :size="15" /><i
                      :class="{ live: s.writer_active }"
                    ></i></span
                  ><span class="source-label"
                    ><b>{{ s.alias || s.owner }}</b
                    ><small
                      >{{ s.owner }}
                      <span v-if="s.parents?.length">· 派生流</span></small
                    ></span
                  ><Icon name="plus" :size="13" /></button
              ></template>
              <div class="source-details">
                <strong>{{ s.alias || s.owner }}</strong>
                <p>{{ s.description || "未填写说明" }}</p>
                <dl>
                  <dt>写入状态</dt>
                  <dd>{{ s.writer_active ? "正在写入" : "空闲" }}</dd>
                  <dt>缓存</dt>
                  <dd>
                    {{ s.buffer_records }} 条 ·
                    {{ Math.round(s.buffer_bytes / 1024) }} KiB
                  </dd>
                  <dt>流 ID</dt>
                  <dd class="mono">{{ s.id }}</dd>
                  <dt>Epoch</dt>
                  <dd class="mono">{{ s.epoch }}</dd>
                </dl>
                <small>点击添加到日志面板</small>
              </div></el-popover
            >
            <p v-if="!streams.length" class="sidebar-hint">
              {{ search ? "没有匹配的流" : "等待输入来源…" }}
            </p>
          </div>
        </div>
        <div class="sidebar-bottom">
          <Icon name="check" :size="13" />页面配置自动保存
        </div>
      </aside>
      <main class="workspace-main">
        <div class="workspace-toolbar">
          <button
            class="icon-button"
            title="切换侧栏"
            aria-label="切换侧栏"
            :aria-pressed="page.sidebar_open"
            @click="pageSet({ sidebar_open: !page.sidebar_open })"
          >
            <Icon name="menu" />
          </button>
          <span class="toolbar-divider"></span
          ><button class="workspace-title" @click="editPage">
            {{ page.title }}<Icon name="more" :size="14" />
          </button>
          <span class="layout-label">{{
            page.layout_mode === "canvas" ? "自由画布" : "网格布局"
          }}</span>
          <div class="toolbar-spacer"></div>
          <div v-if="page.layout_mode === 'canvas'" class="tool-segment">
            <button
              class="icon-button"
              :class="{ active: page.tool === 'select' }"
              title="选择（V）"
              aria-label="选择工具"
              @click="pageSet({ tool: 'select' })"
            >
              <Icon name="select" /></button
            ><button
              class="icon-button"
              :class="{ active: page.tool === 'pan' }"
              title="平移（H）"
              aria-label="平移工具"
              @click="pageSet({ tool: 'pan' })"
            >
              <Icon name="pan" />
            </button>
          </div>
          <button
            class="icon-button"
            :class="{ active: page.locked }"
            :title="page.locked ? '解锁布局' : '锁定布局'"
            :aria-label="page.locked ? '解锁布局' : '锁定布局'"
            @click="pageSet({ locked: !page.locked })"
          >
            <Icon :name="page.locked ? 'lock' : 'unlock'" />
          </button>
          <span class="toolbar-divider"></span>
          <el-dropdown trigger="click" placement="bottom-end"
            ><button class="add-button" aria-label="添加面板">
              <Icon name="plus" :size="15" /><span>添加面板</span></button
            ><template #dropdown
              ><el-dropdown-menu
                ><el-dropdown-item @click="add('log')"
                  ><Icon name="log" />日志监视器</el-dropdown-item
                ><el-dropdown-item @click="add('curve')"
                  ><Icon name="curve" />数值趋势</el-dropdown-item
                ></el-dropdown-menu
              ></template
            ></el-dropdown
          >
          <button
            class="icon-button"
            :class="{ active: page.inspector_open }"
            title="属性面板"
            aria-label="切换属性面板"
            @click="pageSet({ inspector_open: !page.inspector_open })"
          >
            <Icon name="inspector" />
          </button>
          <el-dropdown trigger="click" placement="bottom-end"
            ><button class="icon-button" aria-label="工作台菜单">
              <Icon name="more" /></button
            ><template #dropdown
              ><el-dropdown-menu
                ><el-dropdown-item @click="editPage"
                  >工作台设置</el-dropdown-item
                ><el-dropdown-item
                  @click="
                    pageSet({
                      layout_mode:
                        page.layout_mode === 'canvas' ? 'grid' : 'canvas',
                    })
                  "
                  >切换到{{
                    page.layout_mode === "canvas" ? "网格布局" : "自由画布"
                  }}</el-dropdown-item
                ><el-dropdown-item v-if="page.layout_mode === 'canvas'" :disabled="page.locked" @click="canvas?.compactLayout()"
                  >紧凑排列面板</el-dropdown-item
                ><el-dropdown-item @click="clonePage"
                  >复制工作台</el-dropdown-item
                ><el-dropdown-item
                  :disabled="pages.length < 2"
                  divided
                  @click="removePage"
                  >删除工作台…</el-dropdown-item
                ></el-dropdown-menu
              ></template
            ></el-dropdown
          >
        </div>
        <div class="workspace-notices">
          <div
            v-for="(error, id) in state.errors"
            :key="id"
            class="notice warning"
          >
            <Icon name="warning" />{{ error }}
          </div>
          <div
            v-if="state.archive_writer?.report?.state === 'failed'"
            class="notice danger"
          >
            <Icon name="warning" />归档写入失败：{{
              state.archive_writer.report.error
            }}
          </div>
          <div v-else-if="archiveIssue" class="notice warning">
            <Icon name="warning" />归档已停止，已提交日志仍可查询。
          </div>
        </div>
        <div class="workspace-stage">
          <CanvasBoard
            v-if="page.layout_mode === 'canvas'"
            :key="page.id"
            ref="canvas"
          /><GridBoard v-else :key="page.id" />
          <div v-if="!visiblePanels.length" class="board-empty">
            <span class="empty-art"><Icon name="grid" :size="34" /></span>
            <h1>{{ page.panels.length ? "面板已隐藏" : "开始一个工作台" }}</h1>
            <p>
              {{
                page.panels.length
                  ? "从左侧列表重新显示面板。"
                  : "把日志与趋势放到一起，按你的工作方式排布。"
              }}
            </p>
            <div v-if="!page.panels.length" class="starter-actions">
              <button @click="add('log')">
                <Icon name="log" :size="22" /><strong>日志监视器</strong
                ><span>筛选、搜索、上下文</span></button
              ><button @click="starter">
                <Icon name="curve" :size="22" /><strong>日志与趋势</strong
                ><span>并排查看数据与曲线</span>
              </button>
            </div>
            <button
              v-else
              class="text-button"
              @click="pageSet({ sidebar_open: true })"
            >
              打开面板列表
            </button>
          </div>
          <div v-if="page.layout_mode === 'canvas'" class="canvas-controls">
            <button
              class="icon-button"
              aria-label="缩小画布"
              title="缩小（−）"
              @click="canvas?.zoom(1 / 1.2)"
            >
              <Icon name="zoomOut" /></button
            ><button
              class="zoom-label"
              title="重置为 100%"
              @click="geometryReset"
            >
              {{ Math.round(page.view_zoom * 100) }}%</button
            ><button
              class="icon-button"
              aria-label="放大画布"
              title="放大（+）"
              @click="canvas?.zoom(1.2)"
            >
              <Icon name="zoomIn" /></button
            ><span class="toolbar-divider"></span
            ><button
              class="icon-button"
              aria-label="适应全部面板"
              title="适应全部（0）"
              :disabled="!visiblePanels.length"
              @click="canvas?.fit()"
            >
              <Icon name="fit" /></button
            ><button
              class="icon-button"
              :class="{ active: page.show_minimap }"
              aria-label="切换缩略图"
              title="缩略图"
              @click="pageSet({ show_minimap: !page.show_minimap })"
            >
              <Icon name="grid" />
            </button>
          </div>
          <div
            v-if="page.layout_mode === 'canvas' && !page.show_minimap"
            class="canvas-hint"
          >
            {{
              page.locked
                ? "布局已锁定"
                : page.tool === "pan"
                  ? "拖动画布平移"
                  : "拖动标题移动 · 选中边缘调整尺寸"
            }}<kbd>H</kbd><span>平移</span>
          </div>
        </div>
        <footer class="workspace-status">
          <span
            ><span class="status-dot" :class="{ offline: !online }"></span
            >{{ online ? "本地连接正常" : "连接中断，正在重试" }}</span
          ><span class="status-save" :class="{ 'warning-text': saveError }" :title="saveError"
            ><Icon :name="saving ? 'refresh' : saveError ? 'warning' : 'check'" :size="12" />{{
              saving ? "正在保存…" : saveError ? "部分更改未保存" : "所有更改已保存"
            }}</span
          ><span v-if="panelDrafts.size" class="warning-text">{{ panelDrafts.size }} 个面板草稿待应用</span><span
            >{{ visiblePanels.length }} / {{ page.panels.length }} 个面板</span
          ><code>rev {{ state.revision }}</code>
        </footer>
      </main>
      <Inspector v-if="page.inspector_open" />
    </template>
    <div v-else class="load-state">
      <span class="loading-line"></span>
      <p>正在打开工作台…</p>
    </div>
    <el-dialog v-model="settings" title="工作台设置" width="440"
      ><el-form label-position="top"
        ><el-form-item label="显示名称"
          ><el-input v-model="pageForm.title" /></el-form-item
        ><el-form-item label="唯一名称（CLI 定位）"
          ><el-input v-model="pageForm.name" /></el-form-item
        ><el-form-item label="外观"
          ><el-radio-group v-model="pageForm.theme"
            ><el-radio-button value="light">浅色</el-radio-button
            ><el-radio-button value="dark"
              >深色</el-radio-button
            ></el-radio-group
          ></el-form-item
        ><el-form-item label="排序"
          ><el-input-number v-model="pageForm.order" /></el-form-item></el-form
      ><template #footer
        ><el-button @click="settings = false">取消</el-button
        ><el-button type="primary" @click="savePage">保存</el-button></template
      ></el-dialog
    >
    <el-dialog v-model="cliHelp" title="使用 CLI 编排工作台" width="700"
      ><p class="dialog-intro">
        AI
        或脚本可通过同一组命令控制位置、尺寸、来源、过滤和视角。提交后会同步到所有浏览器。将
        WEB 替换为插件 ID，PANEL_ID 替换为实际面板 ID。
      </p>
      <pre class="cli-example">{{ cliExample }}</pre>
      <p class="field-help">
        使用 page get / panel get 读取当前状态；通过 --revision 检查并发修改。
      </p>
      <template #footer
        ><el-button @click="cliHelp = false">关闭</el-button
        ><el-button type="primary" @click="copyCli"
          >复制命令</el-button
        ></template
      ></el-dialog
    >
  </div>
</template>
