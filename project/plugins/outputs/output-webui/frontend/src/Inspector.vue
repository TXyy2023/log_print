<script setup lang="ts">
import { computed, inject, ref, watch } from "vue";
import { AppContext, bindingValue, type Data } from "./api";
import Icon from "./Icon.vue";
import { clone, patchFrom, conflictingFields, panelFields, fieldLabels } from "./panelDraft";
import { available } from "./layout";
const ctx = inject(AppContext)!;
const page = computed(() =>
  ctx.state.value.pages.find((p: Data) => p.id === ctx.state.value.selected),
);
const panel = computed(() =>
  page.value.panels.find((p: Data) => p.id === page.value.active_panel),
);
const form = ref<Data>({}), base = ref<Data>({}), error = ref(""), saving = ref(false);
const draftKey = () => `${page.value.id}:${panel.value?.id}`;
let loadedKey = "", loading = false;
const normalized = (p: Data): Data => ({ ...clone(p), streams: p.streams.map((s: Data) => bindingValue(s)) });
const patch = computed(() => patchFrom(base.value, form.value, panelFields));
const dirty = computed(() => Object.keys(patch.value).length > 0);
const conflicts = computed(() => panel.value ? conflictingFields(base.value, patch.value, normalized(panel.value)) : []);
function load(restore = false) {
  loading = true;
  loadedKey = draftKey();
  const draft = restore ? ctx.panelDrafts.value.get(loadedKey) : undefined;
  base.value = draft ? clone(draft.base) : panel.value ? normalized(panel.value) : {};
  form.value = draft ? clone(draft.form) : clone(base.value);
  error.value = draft?.error ?? "";
  if (!draft) ctx.panelDrafts.value.delete(loadedKey);
  loading = false;
}
watch([() => page.value.id, () => panel.value?.id], () => load(true), { immediate: true });
watch(() => JSON.stringify(panel.value), () => { if (!dirty.value && !saving.value) load(); });
watch([form, base, error], () => {
  if (loading || !loadedKey) return;
  if (dirty.value) ctx.panelDrafts.value.set(loadedKey, { base: clone(base.value), form: clone(form.value), error: error.value });
  else ctx.panelDrafts.value.delete(loadedKey);
}, { deep: true, flush: "sync" });
function resolveField(key: string, mine: boolean) {
  const current = normalized(panel.value)[key];
  base.value[key] = clone(current);
  if (!mine) form.value[key] = clone(current);
  error.value = "";
}
function resetDraft() {
  ctx.clearEditError(`panel:${loadedKey}`);
  load();
}
const streams = computed(() => {
  const available = ctx.state.value.streams.map((s: Data) => ({
    label: s.alias || s.owner,
    value: bindingValue(
      s.alias
        ? { owner: s.owner, alias: s.alias }
        : { owner: s.owner, alias: null, stream: s.id, epoch: s.epoch },
    ),
  }));
  for (const binding of panel.value?.streams || []) {
    const value = bindingValue(binding);
    if (!available.some((s: Data) => s.value === value))
      available.push({
        value,
        label: `${binding.alias || binding.owner || binding.stream} · 等待来源`,
      });
  }
  return available;
});
async function save() {
  const pageId = page.value.id, panelId = panel.value.id, key = loadedKey;
  const original = clone(base.value), changes = clone(patch.value);
  const wireChanges = { ...changes };
  if (wireChanges.streams) wireChanges.streams = wireChanges.streams.map((s: string) => JSON.parse(s));
  saving.value = true;
  error.value = "";
  try {
    await ctx.command("panel.set", { page: pageId, panel: panelId, ...wireChanges }, {
      editKey: `panel:${key}`, localError: true, retryConflict: true,
      guard: (state) => {
        const p = state.pages.find((p: Data) => p.id === pageId);
        const target = p?.panels.find((p: Data) => p.id === panelId);
        if (!target || conflictingFields(original, changes, normalized(target)).length) return false;
        if (["left", "top", "panel_width", "panel_height", "hidden"].some(k => k in changes) && !p.allow_overlap && p.layout_mode === "canvas" && !(wireChanges.hidden ?? target.hidden)) {
          if (!available({ ...target, ...wireChanges }, p.panels)) throw new Error("面板位置与其他面板重叠，请调整位置或尺寸。");
        }
        return true;
      },
    });
    ctx.panelDrafts.value.delete(key);
    if (loadedKey === key) load();
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    if (loadedKey === key) error.value = message;
    else { const draft = ctx.panelDrafts.value.get(key); if (draft) draft.error = message; }
  } finally {
    saving.value = false;
  }
}
function pageSet(args: Data) {
  void ctx
    .command("page.set", { page: page.value.id, ...args })
    .catch(() => {});
}
async function layer(front: boolean) {
  const others = [...page.value.panels]
    .filter((p: Data) => p.id !== panel.value.id)
    .sort((a: Data, b: Data) => a.z_index - b.z_index);
  const ordered = front ? [...others, panel.value] : [panel.value, ...others];
  await ctx
    .command("layout.set", {
      page: page.value.id,
      layout: ordered.map((p: Data, z_index: number) => ({
        id: p.id,
        z_index,
      })),
    })
    .catch(() => {});
}
</script>
<template>
  <aside class="inspector" aria-label="属性面板">
    <div class="rail-header">
      <strong>{{ panel ? "面板属性" : "画布设置" }}</strong
      ><button
        class="icon-button"
        aria-label="关闭属性面板"
        @click="pageSet({ inspector_open: false })"
      >
        <Icon name="close" />
      </button>
    </div>
    <template v-if="panel">
      <div class="inspector-scroll">
        <div class="object-heading">
          <span class="object-icon"
            ><Icon :name="panel.kind" :size="19"
          /></span>
          <div>
            <strong>{{ panel.kind === "log" ? "日志面板" : "曲线面板" }}</strong
            ><code>{{ panel.id.slice(0, 8) }}</code>
          </div>
          <span v-if="dirty" class="unsaved-dot" title="有未保存的更改"></span>
        </div>
        <el-form
          label-position="top"
          size="small"
          class="inspector-form"
          :disabled="saving"

        >
          <el-form-item label="名称"
            ><el-input v-model="form.title"
          /></el-form-item>
          <div class="property-section">
            <h3>
              位置与尺寸
              <span>{{ page.layout_mode === "canvas" ? "px" : "grid" }}</span>
            </h3>
            <div v-if="page.layout_mode === 'canvas'" class="property-grid">
              <label
                >X<el-input-number
                  v-model="form.left"
                  :controls="false"
                  :min="-1000000"
                  :max="1000000"
                   /></label
              ><label
                >Y<el-input-number
                  v-model="form.top"
                  :controls="false"
                  :min="-1000000"
                  :max="1000000"
                   /></label
              ><label
                >W<el-input-number
                  v-model="form.panel_width"
                  :controls="false"
                  :min="320"
                  :max="4000"
                   /></label
              ><label
                >H<el-input-number
                  v-model="form.panel_height"
                  :controls="false"
                  :min="220"
                  :max="4000"

              /></label>
            </div>
            <div v-else class="property-grid">
              <label
                >X<el-input-number
                  v-model="form.x"
                  :controls="false"
                  :min="0"
                  :max="11"
                   /></label
              ><label
                >Y<el-input-number
                  v-model="form.y"
                  :controls="false"
                  :min="0"
                   /></label
              ><label
                >W<el-input-number
                  v-model="form.w"
                  :controls="false"
                  :min="1"
                  :max="12"
                   /></label
              ><label
                >H<el-input-number
                  v-model="form.h"
                  :controls="false"
                  :min="1"
                  :max="100"

              /></label>
            </div>
            <div class="property-toggle">
              <span>锁定位置</span
              ><el-switch v-model="form.locked"  />
            </div>
            <div class="property-toggle">
              <span>隐藏面板</span
              ><el-switch v-model="form.hidden"  />
            </div>
            <div v-if="page.allow_overlap && page.layout_mode === 'canvas'" class="button-pair">
              <el-button :disabled="dirty" @click="layer(false)"
                >移到底层</el-button
              ><el-button :disabled="dirty" @click="layer(true)"
                >移到顶层</el-button
              >
            </div>
          </div>
          <div class="property-section">
            <h3>数据来源</h3>
            <el-select
              v-model="form.streams"
              multiple
              filterable
              placeholder="全部来源"

              ><el-option
                v-for="s in streams"
                :key="s.value"
                :label="s.label"
                :value="s.value" /></el-select
            ><span class="field-help">留空显示全部流；来源中断时保留绑定。</span
            ><el-form-item label="通道"
              ><el-select
                v-model="form.channels"
                multiple
                allow-create
                filterable
                placeholder="全部通道"

                ><el-option value="stdout" label="stdout" /><el-option
                  value="stderr"
                  label="stderr" /></el-select
            ></el-form-item>
          </div>
          <div v-if="panel.kind === 'log'" class="property-section">
            <h3>日志显示</h3>
            <el-radio-group v-model="form.format"
              ><el-radio-button value="text">文本</el-radio-button
              ><el-radio-button value="hex"
                >十六进制</el-radio-button
              ></el-radio-group
            >
            <div class="property-grid typography">
              <label
                >字号<el-input-number
                  v-model="form.font_size"
                  :min="10"
                  :max="24"
                  :controls="false"
                   /></label
              ><label
                >行高<el-input-number
                  v-model="form.row_height"
                  :min="22"
                  :max="56"
                  :controls="false"

              /></label>
            </div>
            <div class="property-toggle">
              <span>显示元数据</span
              ><el-switch v-model="form.metadata"  />
            </div>
            <div class="property-toggle">
              <span>跟随最新记录</span
              ><el-switch v-model="form.follow"  />
            </div>
          </div>
        </el-form>
      </div>
      <div v-if="conflicts.length || error" class="draft-feedback" role="alert">
        <p v-if="error">{{ error }}</p>
        <div v-for="key in conflicts" :key="key" class="field-conflict">
          <strong>{{ fieldLabels[key] || key }}已被修改</strong>
          <span>当前：{{ normalized(panel)[key] }} · 我的：{{ form[key] }}</span>
          <el-button size="small" @click="resolveField(key, false)">采用当前值</el-button>
          <el-button size="small" @click="resolveField(key, true)">保留我的修改</el-button>
        </div>
      </div>
      <div class="inspector-footer">
        <span>{{ dirty ? "更改尚未应用" : "已保存" }}</span
        ><el-button size="small" :disabled="!dirty || saving" @click="resetDraft"
          >还原</el-button
        ><el-button
          size="small"
          type="primary"
          :loading="saving"
          :disabled="!dirty || conflicts.length > 0"
          @click="save"
          >应用</el-button
        >
      </div>
    </template>
    <div v-else class="inspector-scroll">
      <div class="property-section">
        <h3>布局方式</h3>
        <el-radio-group
          :model-value="page.layout_mode"
          size="small"
          @change="
            (v: string | number | boolean) => pageSet({ layout_mode: v })
          "
          ><el-radio-button value="canvas">自由画布</el-radio-button
          ><el-radio-button value="grid"
            >网格布局</el-radio-button
          ></el-radio-group
        >
        <p class="field-help">
          自由画布默认平铺避让，支持缩放与平移；网格布局自动整理窗口。
        </p>
      </div>
      <div class="property-section">
        <h3>外观</h3>
        <el-radio-group
          :model-value="page.theme"
          size="small"
          @change="(v: string | number | boolean) => pageSet({ theme: v })"
          ><el-radio-button value="light">浅色</el-radio-button
          ><el-radio-button value="dark">深色</el-radio-button></el-radio-group
        >
      </div>
      <div class="property-section">
        <h3>画布辅助</h3>
        <div class="property-toggle">
          <span>显示网点</span
          ><el-switch
            :model-value="page.show_grid"
            @change="
              (v: string | number | boolean) => pageSet({ show_grid: v })
            "
          />
        </div>
        <div class="property-toggle">
          <span>对齐网格</span
          ><el-switch
            :model-value="page.snap"
            @change="(v: string | number | boolean) => pageSet({ snap: v })"
          />
        </div>
        <div class="property-toggle">
          <span>显示缩略图</span
          ><el-switch
            :model-value="page.show_minimap"
            @change="
              (v: string | number | boolean) => pageSet({ show_minimap: v })
            "
          />
        </div>
        <div class="property-toggle">
          <span>锁定布局</span
          ><el-switch
            :model-value="page.locked"
            @change="(v: string | number | boolean) => pageSet({ locked: v })"
          />
        </div>
      </div>
      <div class="inspector-hint">
        <Icon name="select" />
        <p>点击窗口标题或左侧面板列表，编辑它的位置、尺寸与显示内容。</p>
      </div>
    </div>
  </aside>
</template>
