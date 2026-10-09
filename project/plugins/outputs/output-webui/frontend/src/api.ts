import type { Ref, InjectionKey } from "vue";
import type { PanelDraft } from "./panelDraft";
export type Data = Record<string, any>;
// Rust's JSON map order differs from browser object insertion order.
// Stable option values keep restored owner/alias bindings selected correctly.
export function bindingValue(binding: Data): string {
  return JSON.stringify(binding, Object.keys(binding).sort());
}
export interface Context {
  state: Ref<Data>;
  deferredEdits: Ref<number>;
  panelDrafts: Ref<Map<string, PanelDraft>>;
  clearEditError: (key: string) => void;
  command: (method: string, args?: Data, options?: CommandOptions) => Promise<Data>;
  refresh: () => Promise<void>;
  selectPanel: (id: string | null, inspect?: boolean) => Promise<void>;
}
export interface CommandOptions {
  // Evaluate after earlier edits have committed, before attaching the revision.
  guard?: (state: Data) => boolean;
  // Only guarded field patches may rebase after a server revision conflict.
  retryConflict?: boolean;
  localError?: boolean;
  editKey?: string;
}
export class RevisionConflict extends Error {
  constructor() {
    super("配置版本已变化，本次修改未保存。请基于最新配置重新操作。");
  }
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
      throw new RevisionConflict();
    throw new Error(value.error || "请求失败");
  }
  return value;
}
