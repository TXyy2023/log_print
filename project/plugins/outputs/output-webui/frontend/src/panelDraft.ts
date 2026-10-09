type Data = Record<string, any>;
export const clone = <T>(value: T): T => value === undefined ? value : JSON.parse(JSON.stringify(value));
export function equal(a: any, b: any): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length && keys.every(k => equal(a[k], b[k]));
}
export function patchFrom(base: Data, form: Data, fields: readonly string[]): Data {
  return Object.fromEntries(fields.filter(k => !equal(base[k], form[k])).map(k => [k, clone(form[k])]));
}
export function conflictingFields(base: Data, patch: Data, latest: Data): string[] {
  return Object.keys(patch).filter(k => !equal(base[k], latest[k]) && !equal(patch[k], latest[k]));
}
export const panelFields = ['title', 'left', 'top', 'panel_width', 'panel_height', 'x', 'y', 'w', 'h',
  'hidden', 'locked', 'font_size', 'row_height', 'streams', 'channels', 'format', 'metadata', 'follow'] as const;
export const fieldLabels: Record<string, string> = {
  title: '名称', left: 'X', top: 'Y', panel_width: '宽度', panel_height: '高度',
  x: '网格 X', y: '网格 Y', w: '网格宽度', h: '网格高度', hidden: '隐藏面板', locked: '锁定位置',
  font_size: '字号', row_height: '行高', streams: '数据来源', channels: '通道', format: '日志格式',
  metadata: '元数据', follow: '跟随记录', regex: '正则过滤', columns: '可见列', legend: '图例',
  y_min: 'Y 最小值', y_max: 'Y 最大值', time_from: '开始时间', time_end: '结束时间',
};
export interface PanelDraft { base: Data; form: Data; error: string }
