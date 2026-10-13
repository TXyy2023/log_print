import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue';
import { call } from '../api';
import { hasGaps, rowKey } from '../display';
import { LIMITS, type Data, type LogRow, type SessionState } from '../types';

/** Owns transport and query lifecycles. Display components never call the API. */
export function useLogSession() {
  const state = shallowRef<SessionState>({ streams: [], imports: [], recordings: [] });
  const selected = ref(localStorage.getItem('log-print.stream') || '');
  const rows = shallowRef<LogRow[]>([]);
  const keyword = ref('');
  const onlyMatches = ref(false);
  const paused = ref(false);
  const follow = ref(true);
  const connection = ref<'connecting' | 'online' | 'offline'>('connecting');
  const error = ref('');
  const loading = ref(true);
  const uploadBusy = ref(false);
  const uploadName = ref('');
  const saveBusy = ref(false);
  const historyBusy = ref(false);
  const pageBusy = ref(false);
  const historyMode = ref(false);
  const query = ref('');
  const offset = ref(0);
  const total = ref(0);
  const nextOffset = ref(0);
  const pageOffsets = ref<number[]>([]);
  const retained = ref(0);
  const coverage = shallowRef<Data | null>(null);
  const selectedRow = shallowRef<LogRow | null>(null);
  const contextRows = shallowRef<LogRow[]>([]);
  const contextBusy = ref(false);
  const contextError = ref('');
  const imported = computed(() => state.value.imports.find(i => i.stream === selected.value));
  const stream = computed(() => state.value.streams.find(s => s.id === selected.value));
  const recordings = computed(() => state.value.recordings.filter(r => !state.value.imports.some(i => i.recording === r.id)).sort((a, b) => Number(b.created_at) - Number(a.created_at)));
  const activeRecordings = computed(() => recordings.value.filter(r => ['starting', 'recording', 'stopping'].includes(r.state)));
  const filtered = computed(() => onlyMatches.value && keyword.value ? rows.value.filter(r => r.text.includes(keyword.value)) : rows.value);
  const missing = computed(() => hasGaps(coverage.value));
  const needsAttention = computed(() => state.value.archive_writer?.report?.error || (state.value.archive_enabled && state.value.archive_writer?.connected === false ? '配套归档已停止，实时采集仍在继续。' : ''));
  const message = (e: unknown) => e instanceof Error ? e.message : String(e);
  let events: EventSource | undefined;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let generation = 0;
  let contextGeneration = 0;
  let busy = false;
  let disposed = false;
  let tailKey = '';
  let cursor = '';
  const canOperate = computed(() => connection.value === 'online');

  function apply(value: SessionState) {
    state.value = value;
    if (!value.streams.some(s => s.id === selected.value) && !value.imports.some(i => i.stream === selected.value)) selected.value = value.streams[0]?.id || '';
  }
  async function refresh() {
    const response = await fetch('/api/state');
    if (!response.ok) throw new Error(`无法连接日志服务 (${response.status})`);
    apply(await response.json());
  }
  async function tail() {
    if (disposed || busy || !selected.value || paused.value || historyMode.value) return;
    busy = true;
    const version = generation;
    try {
      const text = onlyMatches.value ? keyword.value : '';
      const key = JSON.stringify([selected.value, text]);
      const data = await call('logs.tail', { stream: selected.value, text, limit: LIMITS.liveRows, ...(key === tailKey ? { since: cursor } : {}) });
      if (version !== generation || paused.value || historyMode.value) return;
      if (data.error) error.value = data.error;
      if (data.unchanged) return;
      tailKey = key; cursor = data.cursor;
      retained.value = data.retained;
      rows.value = data.rows;
    } catch (e) { if (version === generation) error.value = message(e); }
    finally { busy = false; }
  }
  async function poll() { await tail(); if (!disposed) pollTimer = setTimeout(poll, connection.value === 'offline' ? 1000 : LIMITS.pollMs); }
  async function cancelQuery() {
    const previous = query.value; query.value = '';
    if (previous) await call('query.cancel', { query: previous }).catch(() => {});
  }
  function inspect(row: LogRow | null) {
    contextGeneration++; contextRows.value = []; contextError.value = ''; contextBusy.value = false;
    selectedRow.value = row;
  }
  watch(selected, () => {
    generation++; tailKey = ''; historyBusy.value = false; pageBusy.value = false;
    void cancelQuery(); inspect(null);
    rows.value = []; error.value = ''; coverage.value = null;
    paused.value = false; historyMode.value = false; follow.value = true;
    offset.value = 0; pageOffsets.value = [];
    localStorage.setItem('log-print.stream', selected.value);
    if (imported.value?.state === 'complete') void history(); else void tail();
  });
  watch(() => imported.value?.state, value => { if (value === 'complete' && !paused.value && !historyMode.value) void history(); });
  watch([keyword, onlyMatches], (current, previous) => {
    if (historyMode.value && !current[1] && current[1] === previous[1]) return;
    generation++;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => { if (historyMode.value) void history(); else void tail(); }, LIMITS.searchDelayMs);
  });
  function togglePause() {
    if (!selected.value || historyMode.value) return;
    generation++; paused.value = !paused.value;
    if (!paused.value) { follow.value = true; void tail(); }
  }
  function showPage(data: Data, start: number) {
    offset.value = start; total.value = data.total; nextOffset.value = data.next;
    coverage.value = data.status?.coverage || null; rows.value = data.rows;
  }
  async function history() {
    if (!selected.value) return;
    const version = ++generation;
    historyMode.value = true; historyBusy.value = true; error.value = ''; pageOffsets.value = [];
    try {
      await cancelQuery();
      const result = await call('history.read', { text_only: true, streams: [selected.value], text: onlyMatches.value ? keyword.value : '' });
      if (version !== generation || disposed) { await call('query.cancel', { query: result.query }); return; }
      query.value = result.query;
      for (;;) {
        const data = await call('query.get', { query: result.query, offset: 0, limit: LIMITS.pageRows });
        if (version !== generation || disposed) return;
        if (data.status.state === 'failed') throw new Error(data.status.error || '历史读取失败');
        if (data.status.state === 'cancelled') return;
        if (data.status.state !== 'running') { showPage(data, 0); break; }
        await new Promise(resolve => setTimeout(resolve, LIMITS.pollMs));
      }
    } catch (e) { if (version === generation) error.value = message(e); }
    finally { if (version === generation) historyBusy.value = false; }
  }
  async function page(direction: number) {
    if (pageBusy.value) return;
    const version = generation;
    const target = direction > 0 ? nextOffset.value : pageOffsets.value.at(-1) ?? 0;
    pageBusy.value = true;
    try {
      const data = await call('query.get', { query: query.value, offset: target, limit: LIMITS.pageRows });
      if (version !== generation) return;
      if (direction > 0) pageOffsets.value.push(offset.value); else pageOffsets.value.pop();
      showPage(data, target);
    } catch (e) { if (version === generation) error.value = message(e); }
    finally { if (version === generation) pageBusy.value = false; }
  }
  function live() {
    generation++; tailKey = ''; historyMode.value = false; paused.value = false; historyBusy.value = false;
    coverage.value = null; follow.value = true; void cancelQuery(); void tail();
  }
  async function upload(file: File) {
    if (uploadBusy.value) return;
    error.value = '';
    if (file.size > LIMITS.uploadBytes) { error.value = '文件超过 64 MiB。请选择较小的日志文件。'; return; }
    uploadBusy.value = true; uploadName.value = file.name;
    try {
      const response = await fetch('/api/import', { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': encodeURIComponent(file.name) }, body: file });
      const value = await response.json();
      if (!response.ok) throw new Error(value.error || `上传失败 (${response.status})`);
      await refresh(); selected.value = value.stream;
    } catch (e) { error.value = message(e); }
    finally { uploadBusy.value = false; uploadName.value = ''; }
  }
  async function save(mode: string) {
    saveBusy.value = true; error.value = '';
    try { await call('recording.start', { stream: selected.value, mode }); await refresh(); return true; }
    catch (e) { error.value = message(e); return false; }
    finally { saveBusy.value = false; }
  }
  async function stop(id: string) {
    saveBusy.value = true; error.value = '';
    try { const result = await call('recording.stop', { id }); await refresh(); if (result.state === 'failed') throw new Error(result.error); }
    catch (e) { error.value = message(e); }
    finally { saveBusy.value = false; }
  }
  async function context() {
    const row = selectedRow.value;
    if (!row) return;
    const version = ++contextGeneration;
    contextBusy.value = true; contextError.value = '';
    let id = '';
    try {
      const result = await call('history.context', { streams: [row.stream], epoch: row.epoch, seq: row.seq, byte_offset: row.offset, before: 8, after: 8, text_only: true });
      id = result.query;
      for (;;) {
        const data = await call('query.get', { query: id, limit: LIMITS.pageRows });
        if (disposed || version !== contextGeneration) return;
        if (data.status.state === 'failed') throw new Error(data.status.error || '无法读取上下文');
        if (data.status.state !== 'running') {
          contextRows.value = data.rows;
          if (!data.rows.length) contextError.value = '该位置已不在可读范围内。';
          break;
        }
        await new Promise(resolve => setTimeout(resolve, LIMITS.pollMs));
      }
    } catch (e) { if (version === contextGeneration) contextError.value = message(e); }
    finally {
      if (id) void call('query.cancel', { query: id }).catch(() => {});
      if (version === contextGeneration) contextBusy.value = false;
    }
  }
  const selectedIndex = computed(() => selectedRow.value ? filtered.value.findIndex(r => rowKey(r) === rowKey(selectedRow.value!)) : -1);
  onMounted(async () => {
    try { await refresh(); } catch (e) { error.value = message(e); }
    finally { loading.value = false; }
    if (disposed) return;
    events = new EventSource('/api/events');
    events.addEventListener('state', event => { connection.value = 'online'; apply(JSON.parse((event as MessageEvent).data)); });
    events.onerror = () => { connection.value = 'offline'; };
    void poll();
  });
  onBeforeUnmount(() => { disposed = true; generation++; contextGeneration++; events?.close(); clearTimeout(pollTimer); clearTimeout(searchTimer); void cancelQuery(); });
  return { state, selected, rows, keyword, onlyMatches, paused, follow, connection, error, loading, uploadBusy, uploadName, saveBusy, historyBusy, pageBusy, historyMode, offset, total, nextOffset, retained, coverage, selectedRow, selectedIndex, contextRows, contextBusy, contextError, imported, stream, recordings, activeRecordings, filtered, missing, needsAttention, canOperate, togglePause, history, page, live, upload, save, stop, inspect, context };
}
