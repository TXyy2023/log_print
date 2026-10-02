<script setup lang="ts">
import {
  computed,
  onMounted,
  onBeforeUnmount,
  provide,
  ref,
  markRaw,
  watch,
  shallowRef,
} from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { GridStack, type GridStackOptions } from "gridstack/dist/vue";
import type { GridStackNode, GridStack as CoreGridStack } from "gridstack";
import Panel from "./Panel.vue";
import { AppContext, call, type Data } from "./api";
const state = ref<Data>({ pages: [], streams: [], revision: 0 });
const online = ref(false),
  busy = ref(false),
  dialog = ref(false),
  editing = ref<Data>({});
const grid = ref<{ getGrid: () => CoreGridStack | null }>();
const page = computed(() =>
  state.value.pages.find((p: Data) => p.id === state.value.selected),
);
const pages = computed(() =>
  [...state.value.pages].sort((a: Data, b: Data) => a.order - b.order),
);
const components = { Panel: markRaw(Panel) };
const options = shallowRef<GridStackOptions>({});
function configureLayout() {
  options.value = {
    column: 12,
    mode: "float",
    cellHeight: 72,
    margin: 8,
    handle: ".panel-drag",
    resizable: { handles: "se,sw" },
    children: (page.value?.panels || []).map((p: Data) => ({
      id: p.id,
      x: p.x,
      y: p.y,
      w: p.w,
      h: p.h,
      component: "Panel",
      props: { id: p.id },
    })),
  };
}
watch(
  () =>
    JSON.stringify([
      page.value?.id,
      (page.value?.panels || []).map((p: Data) => [p.id, p.x, p.y, p.w, p.h]),
    ]),
  configureLayout,
  { immediate: true },
);
async function refresh() {
  const response = await fetch("/api/state");
  state.value = await response.json();
  document.documentElement.classList.toggle(
    "dark",
    page.value?.theme !== "light",
  );
}
async function command(method: string, args: Data = {}) {
  busy.value = true;
  try {
    const result = await call(method, {
      revision: state.value.revision,
      ...args,
    });
    if (result.state?.pages) state.value = { ...state.value, ...result.state };
    else if (
      ![
        "panel.data",
        "query.get",
        "history.read",
        "history.search",
        "history.context",
        "history.curve",
      ].includes(method)
    )
      await refresh();
    return result;
  } catch (e) {
    ElMessage.error(String(e));
    await refresh();
    throw e;
  } finally {
    busy.value = false;
  }
}
provide(AppContext, { state, command, refresh });
let source: EventSource | undefined;
onMounted(async () => {
  await refresh();
  source = new EventSource("/api/events");
  source.addEventListener("state", (e) => {
    const next = JSON.parse((e as MessageEvent).data);
    state.value = next;
    online.value = true;
    document.documentElement.classList.toggle(
      "dark",
      page.value?.theme !== "light",
    );
  });
  source.onerror = () => (online.value = false);
});
onBeforeUnmount(() => source?.close());
async function createPage() {
  try {
    const result = await ElMessageBox.prompt(
      "给这个 Page 一个唯一名称",
      "创建 Page",
      { inputPlaceholder: "例如：生产服务" },
    );
    await command("page.create", { name: result.value, title: result.value });
  } catch (e) {
    if (!["cancel", "close"].includes(String(e))) throw e;
  }
}
async function clonePage() {
  try {
    const result = await ElMessageBox.prompt("新 Page 名称", "复制 Page");
    await command("page.clone", { page: page.value.id, name: result.value });
  } catch (e) {
    if (!["cancel", "close"].includes(String(e))) throw e;
  }
}
function editPage() {
  editing.value = { ...page.value, revision: state.value.revision };
  dialog.value = true;
}
async function savePage() {
  try {
    await command("page.set", {
      page: editing.value.id,
      name: editing.value.name,
      title: editing.value.title,
      theme: editing.value.theme,
      order: editing.value.order,
      revision: editing.value.revision,
    });
    dialog.value = false;
  } catch {
    /* command already displays the error; keep the form open. */
  }
}
async function removePage() {
  try {
    await ElMessageBox.confirm("删除当前 Page 及其面板配置？", "删除 Page");
    await command("page.delete", { page: page.value.id });
  } catch (e) {
    if (!["cancel", "close"].includes(String(e))) throw e;
  }
}
async function add(kind: string) {
  await command("panel.add", {
    page: page.value.id,
    kind,
    title: kind === "log" ? "日志 · 实时" : "曲线 · 数值",
    x: 0,
    y: Math.max(0, ...page.value.panels.map((p: Data) => p.y + p.h)),
    w: 12,
    h: 6,
  });
}
let layoutRevision: number | undefined;
function beginLayout() {
  layoutRevision = state.value.revision;
}
async function layout() {
  const nodes = grid.value?.getGrid()?.engine.nodes || [];
  try {
    await command("layout.set", {
      page: page.value.id,
      revision: layoutRevision,
      layout: nodes.map((n: GridStackNode) => ({
        id: n.id,
        x: n.x,
        y: n.y,
        w: n.w,
        h: n.h,
      })),
    });
  } catch {
    configureLayout();
  } finally {
    layoutRevision = undefined;
  }
}
</script>
<template>
  <div class="shell">
    <aside class="sidebar">
      <div class="brand">
        <span class="brand-mark">lp</span>
        <div>
          <strong>log-print</strong>
          <small>LOCAL OBSERVATORY</small>
        </div>
      </div>
      <div class="section-label">
        PAGES
        <el-button text size="small" @click="createPage">＋ 新建</el-button>
      </div>
      <el-menu
        :default-active="state.selected"
        @select="(id: string) => command('page.select', { page: id })"
      >
        <el-menu-item v-for="p in pages" :key="p.id" :index="p.id">
          <span class="page-dot">◈</span>
          {{ p.title }}
        </el-menu-item>
      </el-menu>
      <div class="section-label streams-label">
        流目录
        <el-tag size="small" type="info">{{ state.streams.length }}</el-tag>
      </div>
      <div class="stream-card" v-for="s in state.streams" :key="s.id">
        <div>
          <span :class="['status-dot', { active: s.writer_active }]"></span>
          <b>{{ s.alias || s.owner }}</b>
          <el-tag v-if="s.parents.length" size="small" type="warning">
            派生
          </el-tag>
        </div>
        <p>{{ s.description || "暂无说明" }}</p>
        <small>
          {{ s.owner }} · {{ s.buffer_records }} 条 /
          {{ Math.round(s.buffer_bytes / 1024) }} KiB
        </small>
        <el-tooltip :content="`${s.id} · epoch ${s.epoch}`">
          <code>{{ s.id.slice(0, 16) }}…</code>
        </el-tooltip>
      </div>
      <footer>
        <span :class="['status-dot', { active: online }]"></span>
        {{ online ? "已连接 · 多窗口同步" : "连接中断" }}
        <small>
          {{
            state.archive_enabled
              ? "SQLite 归档已配置"
              : "仅实时缓存 · 无全量历史"
          }}
        </small>
      </footer>
    </aside>
    <main>
      <header class="topbar">
        <div>
          <div class="eyebrow">WORKSPACE / {{ page?.name }}</div>
          <h1>{{ page?.title || "加载中" }}</h1>
        </div>
        <div class="top-actions">
          <el-tag type="info">rev {{ state.revision }}</el-tag>
          <el-button @click="editPage">页面设置</el-button>
          <el-dropdown>
            <el-button>添加面板 ＋</el-button>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item @click="add('log')">
                  日志表格
                </el-dropdown-item>
                <el-dropdown-item @click="add('curve')">
                  数值曲线
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
          <el-dropdown>
            <el-button text>•••</el-button>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item @click="clonePage">
                  复制 Page
                </el-dropdown-item>
                <el-dropdown-item @click="removePage">
                  删除 Page
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
        </div>
      </header>
      <div class="workspace-note">
        <span>所有流，一处观察。</span>
        <span>
          {{
            state.archive_enabled
              ? "历史查询固定提交水位，实时与归档合并去重"
              : "未启用归档；可查询范围受内存缓存限制"
          }}
        </span>
      </div>
      <el-alert
        v-for="(error, stream) in state.errors"
        :key="stream"
        type="warning"
        :title="`${stream}: ${error}`"
        :closable="false"
      />
      <el-alert
        v-if="state.archive_writer?.report?.state === 'failed'"
        type="error"
        :title="`归档写入失败：${state.archive_writer.report.error}`"
        :closable="false"
      />
      <el-alert
        v-else-if="
          state.archive_enabled && state.archive_writer?.connected === false
        "
        type="warning"
        title="归档未运行：可读取已提交范围，后续记录尚未归档"
        :closable="false"
      />
      <GridStack
        v-if="page"
        :key="page.id"
        ref="grid"
        :options="options"
        :components="components"
        @dragstop="layout"
        @resizestop="layout"
        @dragstart="beginLayout"
        @resizestart="beginLayout"
      >
        <template #empty>
          <div class="empty">
            <div class="empty-icon">◫</div>
            <h2>为日志留一块画布</h2>
            <p>添加日志或曲线面板。页面、过滤条件和布局会自动保存。</p>
            <el-button type="primary" @click="add('log')">
              添加日志面板
            </el-button>
            <el-button @click="add('curve')">添加曲线面板</el-button>
          </div>
        </template>
      </GridStack>
    </main>
    <el-dialog v-model="dialog" title="Page 设置" width="440">
      <el-form label-position="top">
        <el-form-item label="唯一名称">
          <el-input v-model="editing.name" />
        </el-form-item>
        <el-form-item label="标题">
          <el-input v-model="editing.title" />
        </el-form-item>
        <el-form-item label="主题">
          <el-select v-model="editing.theme">
            <el-option value="dark" label="深色" />
            <el-option value="light" label="浅色" />
          </el-select>
        </el-form-item>
        <el-form-item label="顺序">
          <el-input-number v-model="editing.order" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button type="primary" @click="savePage">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>
