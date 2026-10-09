import type { ViewportTransform } from "@vue-flow/core";

export const sameViewport = (a: ViewportTransform, b: ViewportTransform) =>
  a.x === b.x && a.y === b.y && a.zoom === b.zoom;

export function roundedViewport(value: ViewportTransform): ViewportTransform {
  return {
    x: Math.round(value.x),
    y: Math.round(value.y),
    zoom: Math.round(value.zoom * 1000) / 1000,
  };
}

// macOS pinch can emit start/end for every wheel event. Treat the quiet period
// as the gesture boundary, and retain only the newest view during a slow save.
export function createViewportSaver(options: {
  read: () => ViewportTransform;
  write: (value: ViewportTransform, base: ViewportTransform) => Promise<void>;
  reconcile: () => void;
  pending: (delta: number) => void;
}) {
  let base: ViewportTransform | undefined;
  let latest: ViewportTransform | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let active = false, ready = false, saving = false, rejected = false;
  let disposed = false;

  function clearTimer() {
    clearTimeout(timer);
    timer = undefined;
  }
  function settle() {
    if (base) options.pending(-1);
    base = latest = undefined;
    rejected = ready = false;
    if (!disposed) options.reconcile();
  }
  async function flush() {
    if (saving || active || !base || !ready) return;
    if (rejected || !latest) {
      settle();
      return;
    }
    const value = latest;
    latest = undefined;
    if (sameViewport(value, base)) {
      settle();
      return;
    }
    saving = true;
    try {
      await options.write(value, base);
      base = value;
    } catch {
      // Do not keep submitting the remaining events of a rejected gesture.
      rejected = true;
      latest = undefined;
    } finally {
      saving = false;
      if (ready && !active) {
        if (latest && !rejected) void flush();
        else settle();
      }
    }
  }
  function begin() {
    if (disposed) return;
    if (!base) {
      base = { ...options.read() };
      options.pending(1);
    }
    active = true;
    ready = false;
    clearTimer();
  }
  function change(value: ViewportTransform) {
    if (base && !rejected && !disposed) latest = roundedViewport(value);
  }
  function end(value: ViewportTransform) {
    if (!base || disposed) return;
    change(value);
    active = false;
    clearTimer();
    timer = setTimeout(() => {
      timer = undefined;
      ready = true;
      void flush();
    }, 150);
  }
  return {
    get busy() { return base !== undefined; },
    begin, change, end,
    dispose() {
      disposed = true;
      active = false;
      ready = true;
      clearTimer();
      // Save the final view of the old page, never a newly selected page.
      void flush();
    },
  };
}
