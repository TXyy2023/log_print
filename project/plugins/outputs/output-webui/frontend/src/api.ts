import type { Ref, InjectionKey } from "vue";
export type Data = Record<string, any>;
export interface Context {
  state: Ref<Data>;
  command: (method: string, args?: Data) => Promise<Data>;
  refresh: () => Promise<void>;
}
export const AppContext: InjectionKey<Context> = Symbol("app");
export async function call(method: string, args: Data = {}): Promise<Data> {
  const response = await fetch("/api/control", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ method, args }),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || "Request failed");
  return value;
}
