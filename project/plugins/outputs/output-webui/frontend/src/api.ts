export type { Data } from './types';
import type { Data } from './types';
export async function call(method: string, args: Data = {}): Promise<Data> {
  const response = await fetch('/api/control', {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ method, args }),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || `请求失败 (${response.status})`);
  return value;
}
