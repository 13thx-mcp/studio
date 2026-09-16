import type { LogEntry, ProcessStatus } from "./types";

async function readJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // Keep HTTP status text when the body is not JSON.
    }
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}

export function listServers(): Promise<ProcessStatus[]> {
  return fetch("/api/mcp").then(readJson<ProcessStatus[]>);
}

export function getServer(id: string): Promise<ProcessStatus> {
  return fetch(`/api/mcp/${encodeURIComponent(id)}`).then(readJson<ProcessStatus>);
}

export function getLogs(id: string): Promise<LogEntry[]> {
  return fetch(`/api/mcp/${encodeURIComponent(id)}/logs`).then(readJson<LogEntry[]>);
}

export function lifecycleAction(
  id: string,
  action: "start" | "stop" | "restart",
): Promise<ProcessStatus> {
  return fetch(`/api/mcp/${encodeURIComponent(id)}/${action}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
  }).then(readJson<ProcessStatus>);
}
