export type ProcessState = "stopped" | "starting" | "running" | "stopping" | "failed";

export interface ProcessStatus {
  id: string;
  name: string;
  state: ProcessState;
  pid: number | null;
  uptime_ms: number | null;
  restart_count: number;
  crash_count: number;
  last_exit_code: number | null;
  last_error: string | null;
}

export type LogStream = "stdout" | "stderr" | "studio";

export interface LogEntry {
  sequence: number;
  timestamp_ms: number;
  stream: LogStream;
  message: string;
}

export type TunnelState = ProcessState;

export interface TunnelStatus {
  name: string;
  state: TunnelState;
  runtime_available: boolean;
  pid: number | null;
  uptime_ms: number | null;
  restart_count: number;
  crash_count: number;
  last_exit_code: number | null;
  last_error: string | null;
}

export interface TunnelLogEntry {
  sequence: number;
  timestamp_ms: number;
  stream: LogStream;
  message: string;
}

export type StudioEvent =
  | { type: "snapshot"; servers: ProcessStatus[]; tunnel: TunnelStatus }
  | { type: "process_status"; mcp_id: string; status: ProcessStatus }
  | { type: "log"; mcp_id: string; entry: LogEntry }
  | { type: "tunnel_status"; status: TunnelStatus }
  | { type: "tunnel_log"; entry: TunnelLogEntry }
  | { type: "resync_required" };
