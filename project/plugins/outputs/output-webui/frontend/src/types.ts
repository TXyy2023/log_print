export type Data = Record<string, any>;
export interface StreamInfo { id: string; epoch: string; alias?: string; description?: string; owner: string; head: number; writer_active?: boolean }
export interface LogRow { stream: string; epoch: string; seq: string; offset: number; text: string; time: string; channel?: string; end_seq?: string; partial?: boolean; gap?: boolean; observed_ts_ns?: string; source_ts_ns?: string; context_target?: boolean }
export interface ImportJob { id: string; name: string; stream: string; state: string; bytes: string; recording: string; error?: string }
export interface Recording { id: string; stream: string; state: string; mode: string; path: string; bytes: string; records: string; created_at: string; error?: string; writer?: Data; coverage?: Data }
export interface SessionState { streams: StreamInfo[]; imports: ImportJob[]; recordings: Recording[]; import_enabled?: boolean; archive_enabled?: boolean; archive_writer?: Data }
export const LIMITS = { liveRows: 2000, pageRows: 200, pollMs: 100, searchDelayMs: 150, uploadBytes: 64 * 1024 * 1024, rowHeight: 26, overscan: 8 } as const;
