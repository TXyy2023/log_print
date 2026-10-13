import type { LogRow, StreamInfo } from './types';
export function decodeName(value: string) { try { return decodeURIComponent(value); } catch { return value; } }
export function streamName(stream?: StreamInfo) { return decodeName(stream?.description || stream?.alias || stream?.owner || '选择日志流'); }
export function rowKey(row: LogRow) { return `${row.stream}:${row.epoch}:${row.seq}:${row.offset}`; }
export function fileSize(value: string | number) {
  const bytes = Number(value || 0);
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / (bytes < 1048576 ? 1024 : 1048576)).toFixed(1)} ${bytes < 1048576 ? 'KiB' : 'MiB'}`;
}
export function clockTime(value: string) {
  const date = new Date(Number(value));
  if (!Number.isFinite(date.getTime())) return '—';
  return `${date.toLocaleTimeString('zh-CN', { hour12: false })}.${String(date.getMilliseconds()).padStart(3, '0')}`;
}
export function hasGaps(coverage: any) {
  return Boolean(coverage?.archive_error || coverage?.gap_count || coverage?.streams?.some((s: any) => s.uncovered_prefix || s.uncovered_between || s.uncovered_after || s.memory?.range_count > 1));
}
export function recordingLabel(state: string) { return ({ starting: '准备中', recording: '保存中', stopping: '正在提交', stopped: '已保存', failed: '保存失败' })[state] || state; }
