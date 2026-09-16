import { useEffect, useMemo, useState } from "react";

import { getLogs, lifecycleAction, listServers } from "./api";
import { connectRealtime, type ConnectionState } from "./realtime";
import type { LogEntry, ProcessStatus, StudioEvent } from "./types";
import "./styles.css";

function mergeLogEntries(current: LogEntry[], incoming: LogEntry[]): LogEntry[] {
  const bySequence = new Map<number, LogEntry>();
  for (const entry of current) bySequence.set(entry.sequence, entry);
  for (const entry of incoming) bySequence.set(entry.sequence, entry);
  return [...bySequence.values()].sort((a, b) => a.sequence - b.sequence);
}

function formatUptime(uptimeMs: number | null): string {
  if (uptimeMs === null) return "—";
  const totalSeconds = Math.floor(uptimeMs / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  return [hours, minutes, seconds].map((part) => String(part).padStart(2, "0")).join(":");
}

function actionEnabled(status: ProcessStatus, action: "start" | "stop" | "restart"): boolean {
  if (status.state === "starting" || status.state === "stopping") return false;
  if (action === "start") return status.state === "stopped" || status.state === "failed";
  if (action === "stop") return status.state === "running";
  return status.state === "running" || status.state === "stopped" || status.state === "failed";
}

export default function App() {
  const [servers, setServers] = useState<Record<string, ProcessStatus>>({});
  const [logs, setLogs] = useState<Record<string, LogEntry[]>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [connection, setConnection] = useState<ConnectionState>("connecting");
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const applyEvent = (event: StudioEvent) => {
    if (event.type === "snapshot") {
      setServers(Object.fromEntries(event.servers.map((server) => [server.id, server])));
      setSelectedId((current) => current ?? event.servers[0]?.id ?? null);
      return;
    }
    if (event.type === "process_status") {
      setServers((current) => ({ ...current, [event.mcp_id]: event.status }));
      return;
    }
    if (event.type === "log") {
      setLogs((current) => ({
        ...current,
        [event.mcp_id]: mergeLogEntries(current[event.mcp_id] ?? [], [event.entry]),
      }));
      return;
    }
    if (event.type === "resync_required") {
      void refreshServers();
    }
  };

  const refreshServers = async () => {
    try {
      const result = await listServers();
      setServers(Object.fromEntries(result.map((server) => [server.id, server])));
      setSelectedId((current) => current ?? result[0]?.id ?? null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  useEffect(() => {
    void refreshServers();
    return connectRealtime({ onEvent: applyEvent, onState: setConnection });
  }, []);

  useEffect(() => {
    if (!selectedId) return;
    void getLogs(selectedId)
      .then((entries) => {
        setLogs((current) => ({
          ...current,
          [selectedId]: mergeLogEntries(current[selectedId] ?? [], entries),
        }));
      })
      .catch((cause) => setError(cause instanceof Error ? cause.message : String(cause)));
  }, [selectedId]);

  const serverList = useMemo(
    () => Object.values(servers).sort((a, b) => a.name.localeCompare(b.name)),
    [servers],
  );
  const selected = selectedId ? servers[selectedId] : undefined;
  const selectedLogs = selectedId ? logs[selectedId] ?? [] : [];

  const running = serverList.filter((server) => server.state === "running").length;
  const failed = serverList.filter((server) => server.state === "failed").length;

  const runAction = async (server: ProcessStatus, action: "start" | "stop" | "restart") => {
    setPending(`${server.id}:${action}`);
    setError(null);
    try {
      const updated = await lifecycleAction(server.id, action);
      setServers((current) => ({ ...current, [server.id]: updated }));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  return (
    <main className="shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">Local control plane</p>
          <h1>MCP Studio</h1>
        </div>
        <span className={`connection connection-${connection}`}>{connection}</span>
      </header>

      {error && <div className="error-banner">{error}</div>}

      <section className="summary-grid" aria-label="Studio summary">
        <article className="summary-card"><span>Managed</span><strong>{serverList.length}</strong></article>
        <article className="summary-card"><span>Running</span><strong>{running}</strong></article>
        <article className="summary-card"><span>Failed</span><strong>{failed}</strong></article>
        <article className="summary-card"><span>Restarts</span><strong>{serverList.reduce((sum, server) => sum + server.restart_count, 0)}</strong></article>
      </section>

      <section className="workspace">
        <aside className="server-list">
          <h2>MCP servers</h2>
          {serverList.map((server) => (
            <button
              className={`server-row ${server.id === selectedId ? "selected" : ""}`}
              key={server.id}
              onClick={() => setSelectedId(server.id)}
            >
              <span>
                <strong>{server.name}</strong>
                <small>{server.id}</small>
              </span>
              <span className={`state state-${server.state}`}>{server.state}</span>
            </button>
          ))}
        </aside>

        <section className="detail-panel">
          {!selected ? (
            <div className="empty-state">No MCP server selected.</div>
          ) : (
            <>
              <div className="detail-header">
                <div>
                  <p className="eyebrow">{selected.id}</p>
                  <h2>{selected.name}</h2>
                </div>
                <span className={`state state-${selected.state}`}>{selected.state}</span>
              </div>

              <dl className="metric-grid">
                <div><dt>PID</dt><dd>{selected.pid ?? "—"}</dd></div>
                <div><dt>Uptime</dt><dd>{formatUptime(selected.uptime_ms)}</dd></div>
                <div><dt>Restarts</dt><dd>{selected.restart_count}</dd></div>
                <div><dt>Crashes</dt><dd>{selected.crash_count}</dd></div>
                <div><dt>Last exit</dt><dd>{selected.last_exit_code ?? "—"}</dd></div>
                <div><dt>Last error</dt><dd>{selected.last_error ?? "—"}</dd></div>
              </dl>

              <div className="actions">
                {(["start", "restart", "stop"] as const).map((action) => (
                  <button
                    key={action}
                    disabled={!actionEnabled(selected, action) || pending !== null}
                    onClick={() => void runAction(selected, action)}
                  >
                    {pending === `${selected.id}:${action}` ? `${action}…` : action}
                  </button>
                ))}
              </div>

              <section className="logs">
                <div className="logs-header">
                  <h3>Recent logs</h3>
                  <span>{selectedLogs.length} entries</span>
                </div>
                <div className="log-viewer">
                  {selectedLogs.length === 0 && <p className="log-empty">No logs yet.</p>}
                  {selectedLogs.map((entry) => (
                    <div className={`log-line log-${entry.stream}`} key={entry.sequence}>
                      <time>{new Date(entry.timestamp_ms).toLocaleTimeString()}</time>
                      <span className="log-stream">{entry.stream}</span>
                      <code>{entry.message}</code>
                    </div>
                  ))}
                </div>
              </section>
            </>
          )}
        </section>
      </section>
    </main>
  );
}
