import type { Ref, InjectionKey } from "vue";
export type Data = Record<string, any>;
// Rust's JSON map order differs from browser object insertion order.
// Stable option values keep restored owner/alias bindings selected correctly.
export function bindingValue(binding: Data): string {
  return JSON.stringify(binding, Object.keys(binding).sort());
}
export interface Context {
  state: Ref<Data>;
  command: (method: string, args?: Data) => Promise<Data>;
  refresh: () => Promise<void>;
  selectPanel: (id: string | null, inspect?: boolean) => Promise<void>;
}
export const AppContext: InjectionKey<Context> = Symbol("app");
export async function call(method: string, args: Data = {}): Promise<Data> {
  const response = await fetch("/api/control", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ method, args }),
  });
  const value = await response.json();
  if (!response.ok) {
    if (String(value.error).startsWith("revision_conflict"))
      throw new Error(
        "配置已在其他窗口或 CLI 中更改。请还原后重新编辑，当前修改尚未保存。",
      );
    throw new Error(value.error || "请求失败");
  }
  return value;
}
