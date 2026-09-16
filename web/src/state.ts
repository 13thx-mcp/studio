import type { LogEntry, ProcessStatus } from "./types";

export function mergeLogEntries(current: LogEntry[], incoming: LogEntry[]): LogEntry[] {
  const bySequence = new Map<number, LogEntry>();
  for (const entry of current) bySequence.set(entry.sequence, entry);
  for (const entry of incoming) bySequence.set(entry.sequence, entry);
  return [...bySequence.values()].sort((a, b) => a.sequence - b.sequence);
}

export function formatUptime(uptimeMs: number | null): string {
  if (uptimeMs === null) return "—";
  const totalSeconds = Math.floor(uptimeMs / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  return [hours, minutes, seconds]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}

export function formatLogMessage(message: string): string {
  return message.replace(
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\s+(?:TRACE|DEBUG|INFO|WARN|ERROR)\s+/,
    "",
  );
}

export function actionEnabled(
  status: ProcessStatus,
  action: "start" | "stop" | "restart",
): boolean {
  if (status.state === "starting" || status.state === "stopping") return false;
  if (action === "start") return status.state === "stopped" || status.state === "failed";
  if (action === "stop") return status.state === "running";
  return status.state === "running" || status.state === "stopped" || status.state === "failed";
}
