import type { Data } from "./api";

// Live panels contain display-cache data, not a persistence acknowledgement.
// Only a historical query supplies fixed archive/memory coverage boundaries.
export function coveragePresentation(mode: string, state: Data, coverage?: Data) {
  const live = mode !== "history";
  const writer = live ? state.archive_writer : coverage?.writer;
  const report = writer?.report;
  const failure = (!live && coverage?.archive_error) || report?.error;
  const archiveState =
    report?.state === "failed" || failure
      ? "归档失败"
      : writer?.state === "unavailable"
        ? "归档不可用"
        : writer?.connected === false || report?.state === "stopped"
          ? "归档已停止"
          : report?.state !== "archiving"
            ? "归档未就绪"
            : "";
  if (live) {
    const enabled = state.archive_enabled === true;
    return {
      label: enabled && archiveState ? `实时缓存 · ${archiveState}` : "实时缓存",
      tone: enabled && archiveState ? "warning" : "neutral",
      description: enabled
        ? "这里显示实时缓存，不代表日志已提交。请切换到历史查询核对归档与内存覆盖范围。"
        : "未启用归档。这里仅显示保留的实时缓存，停止实例或缓存覆盖后无法从 Core 恢复。",
      error: enabled ? failure || writer?.error : undefined,
    };
  }
  if (!coverage) {
    return {
      label: "覆盖信息未就绪",
      tone: "neutral",
      description: "尚未取得本次历史查询的覆盖信息，不能确认归档范围。",
    };
  }
  const archived = coverage.mode === "archive_and_memory";
  const streams: Data[] = coverage.streams || [];
  const incomplete =
    coverage.gap_count ||
    coverage.runtime_match === false ||
    streams.some(
      (s) =>
        s.uncovered_prefix ||
        s.uncovered_between ||
        s.uncovered_after ||
        s.memory?.range_count > 1,
    );
  const committed = streams.some((s) => s.archived && !s.archived.empty);
  return {
    label:
      archived && archiveState
        ? archiveState
        : incomplete
          ? "覆盖不完整"
          : !archived
            ? "内存范围"
            : streams.some((s) => s.uncommitted) || !committed
              ? "归档待提交"
              : "归档上下文",
    tone:
      (archived && archiveState) ||
      incomplete ||
      (archived && (streams.some((s) => s.uncommitted) || !committed))
        ? "warning"
        : archived
          ? "success"
          : "neutral",
    description: "覆盖范围固定于本次历史查询；详情同时列出归档与内存，不能将内存中的记录视为已归档。",
    error: failure || writer?.error,
  };
}
