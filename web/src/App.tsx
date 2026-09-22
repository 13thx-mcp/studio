import { useEffect, useMemo, useRef, useState } from "react";

import {
  getLogs,
  getTunnel,
  getTunnelLogs,
  getReconciliation,
  getUpdateTransaction,
  lifecycleAction,
  listDiscovery,
  listRegistry,
  listServers,
  listUpdates,
  checkUpdates,
  prepareUpdate,
  applyUpdate,
  checkReconciliation,
  applyReconciliation,
  registerDiscovery,
  scanDiscovery,
  setRegistryEnabled,
  tunnelLifecycleAction,
  unregisterRegistry,
  updateRegistry,
} from "./api";
import { connectRealtime, type ConnectionState } from "./realtime";
import {
  actionEnabled,
  formatLogMessage,
  formatUptime,
  mergeLogEntries,
  tunnelActionEnabled,
} from "./state";
import UpdatesPanel from "./UpdatesPanel";
import HistoryPanel from "./HistoryPanel";
import { persistPendingTransaction, readPendingTransactions } from "./updates-state";
import type {
  DiscoveredProject,
  LogEntry,
  LogStream,
  ProcessStatus,
  RegistryEntry,
  StudioEvent,
  TunnelLogEntry,
  TunnelStatus,
  UpdateComponent,
  UpdateInventory,
  UpdateTransaction,
  ReconciliationView,
} from "./types";
import "./styles.css";

type LogFilter = "all" | LogStream;

export default function App() {
  const [servers, setServers] = useState<Record<string, ProcessStatus>>({});
  const [logs, setLogs] = useState<Record<string, LogEntry[]>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [registry, setRegistry] = useState<RegistryEntry[]>([]);
  const [discovery, setDiscovery] = useState<DiscoveredProject[]>([]);
  const [tunnel, setTunnel] = useState<TunnelStatus | null>(null);
  const [tunnelLogs, setTunnelLogs] = useState<TunnelLogEntry[]>([]);
  const [updates, setUpdates] = useState<UpdateInventory[]>([]);
  const [reconciliation, setReconciliation] = useState<ReconciliationView | null>(null);
  const [updateTransactions, setUpdateTransactions] = useState<Partial<Record<UpdateComponent, UpdateTransaction>>>({});
  const [connection, setConnection] = useState<ConnectionState>("connecting");
  const [historyRefreshKey, setHistoryRefreshKey] = useState(0);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [logFilter, setLogFilter] = useState<LogFilter>("all");
  const [autoScroll, setAutoScroll] = useState(true);
  const [uptimeTick, setUptimeTick] = useState(0);
  const logViewerRef = useRef<HTMLDivElement | null>(null);
  const tunnelLogViewerRef = useRef<HTMLDivElement | null>(null);

  const refreshRuntime = async () => {
    const [serverResult, tunnelResult, tunnelLogResult] = await Promise.all([
      listServers(),
      getTunnel(),
      getTunnelLogs(),
    ]);
    setServers(Object.fromEntries(serverResult.map((server) => [server.id, server])));
    setSelectedId((current) => current ?? serverResult[0]?.id ?? null);
    setTunnel(tunnelResult);
    setTunnelLogs((current) => mergeLogEntries(current, tunnelLogResult));
  };

  const refreshRegistry = async () => setRegistry(await listRegistry());
  const refreshDiscovery = async () => setDiscovery(await listDiscovery());
  const refreshUpdates = async () => setUpdates(await listUpdates());
  const refreshReconciliation = async () => setReconciliation(await getReconciliation());

  const rememberTransaction = (transaction: UpdateTransaction) => {
    setUpdateTransactions((current) => ({ ...current, [transaction.component]: transaction }));
    persistPendingTransaction(window.localStorage, transaction);
  };

  const refreshUpdateTransaction = async (transactionId: string) => {
    const transaction = await getUpdateTransaction(transactionId);
    rememberTransaction(transaction);
  };

  const refreshAll = async () => {
    try {
      await Promise.all([refreshRuntime(), refreshRegistry(), refreshDiscovery(), refreshUpdates(), refreshReconciliation()]);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const applyEvent = (event: StudioEvent) => {
    if (event.type === "snapshot") {
      setServers(Object.fromEntries(event.servers.map((server) => [server.id, server])));
      setSelectedId((current) => current ?? event.servers[0]?.id ?? null);
      setTunnel(event.tunnel);
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
    if (event.type === "tunnel_status") {
      setTunnel(event.status);
      return;
    }
    if (event.type === "tunnel_log") {
      setTunnelLogs((current) => mergeLogEntries(current, [event.entry]));
      return;
    }
    if (event.type === "registry_changed") {
      void Promise.all([refreshRegistry(), listServers().then((items) => {
        setServers(Object.fromEntries(items.map((server) => [server.id, server])));
      })]);
      return;
    }
    if (event.type === "discovery_changed") {
      void refreshDiscovery();
      return;
    }
    if (event.type === "updates_changed") {
      void refreshUpdates();
      return;
    }
    if (event.type === "update_transaction") {
      rememberTransaction(event.transaction);
      return;
    }
    if (event.type === "reconciliation_changed") {
      setReconciliation(event.status);
      return;
    }
    if (event.type === "history_committed" || event.type === "history_health") {
      setHistoryRefreshKey((current) => current + 1);
      return;
    }
    if (event.type === "resync_required") {
      setHistoryRefreshKey((current) => current + 1);
      void refreshAll();
    }
  };

  useEffect(() => {
    void refreshAll();
    for (const pendingUpdate of readPendingTransactions(window.localStorage)) {
      void refreshUpdateTransaction(pendingUpdate.transaction_id).catch(() => {
        // A later realtime reconnect/resync can retry durable transaction lookup.
      });
    }
    return connectRealtime({ onEvent: applyEvent, onState: setConnection });
  }, []);

  useEffect(() => {
    if (connection !== "connected") return;
    void Promise.all([refreshUpdates(), refreshReconciliation()]);
    for (const pendingUpdate of readPendingTransactions(window.localStorage)) {
      void refreshUpdateTransaction(pendingUpdate.transaction_id).catch((cause) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      });
    }
  }, [connection]);

  useEffect(() => {
    if (!selectedId) return;
    setLogFilter("all");
    setAutoScroll(true);
    void getLogs(selectedId)
      .then((entries) => {
        setLogs((current) => ({
          ...current,
          [selectedId]: mergeLogEntries(current[selectedId] ?? [], entries),
        }));
      })
      .catch((cause) => setError(cause instanceof Error ? cause.message : String(cause)));
  }, [selectedId]);

  useEffect(() => {
    const timer = window.setInterval(() => setUptimeTick((tick) => tick + 1), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const serverList = useMemo(
    () => Object.values(servers).sort((a, b) => a.name.localeCompare(b.name)),
    [servers],
  );
  const selected = selectedId ? servers[selectedId] : undefined;
  const selectedLogs = selectedId ? logs[selectedId] ?? [] : [];
  const visibleLogs = useMemo(
    () => selectedLogs.filter((entry) => logFilter === "all" || entry.stream === logFilter),
    [selectedLogs, logFilter],
  );

  useEffect(() => {
    if (!autoScroll) return;
    const viewer = logViewerRef.current;
    if (viewer) viewer.scrollTop = viewer.scrollHeight;
    const tunnelViewer = tunnelLogViewerRef.current;
    if (tunnelViewer) tunnelViewer.scrollTop = tunnelViewer.scrollHeight;
  }, [visibleLogs, tunnelLogs, autoScroll]);

  const running = serverList.filter((server) => server.state === "running").length;
  const failed = serverList.filter((server) => server.state === "failed").length;

  const runMcpAction = async (server: ProcessStatus, action: "start" | "stop" | "restart") => {
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

  const runUpdateCheck = async () => {
    setPending("updates:check");
    setError(null);
    try {
      setUpdates(await checkUpdates());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const runPrepareUpdate = async (component: UpdateComponent, version: string) => {
    setPending(`updates:${component}:prepare`);
    setError(null);
    try {
      rememberTransaction(await prepareUpdate(component, version));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const runApplyUpdate = async (transaction: UpdateTransaction) => {
    setPending(`updates:${transaction.component}:apply`);
    setError(null);
    persistPendingTransaction(window.localStorage, transaction);
    try {
      rememberTransaction(await applyUpdate(transaction.component, transaction.transaction_id));
      await refreshUpdates();
    } catch (cause) {
      if (transaction.component === "studio" || transaction.component === "gateway") {
        setError("The control-path request disconnected or failed. The transaction ID is retained and will be re-queried after reconnect.");
      } else {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    } finally {
      setPending(null);
    }
  };

  const runRefreshUpdateTransaction = async (transactionId: string) => {
    setPending(`updates:transaction:${transactionId}`);
    setError(null);
    try {
      await refreshUpdateTransaction(transactionId);
      await refreshUpdates();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const runReconciliationCheck = async () => {
    setPending("reconciliation:check");
    setError(null);
    try {
      setReconciliation(await checkReconciliation());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const runReconciliationApply = async () => {
    setPending("reconciliation:apply");
    setError(null);
    try {
      setReconciliation(await applyReconciliation());
      await refreshUpdates();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const runTunnelAction = async (action: "start" | "stop" | "restart") => {
    setPending(`tunnel:${action}`);
    setError(null);
    try {
      setTunnel(await tunnelLifecycleAction(action));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const toggleRegistry = async (entry: RegistryEntry) => {
    setPending(`registry:${entry.id}:toggle`);
    setError(null);
    try {
      await setRegistryEnabled(entry.id, !entry.enabled);
      await Promise.all([refreshRegistry(), refreshRuntime()]);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const editRegistry = async (entry: RegistryEntry) => {
    const name = window.prompt("Display name", entry.name);
    if (name === null) return;
    const executable = window.prompt("Project-relative executable", entry.executable);
    if (executable === null) return;
    const workingDir = window.prompt("Project-relative working directory", entry.working_dir);
    if (workingDir === null) return;
    const argsText = window.prompt("Arguments (one per line)", entry.args.join("\n"));
    if (argsText === null) return;
    setPending(`registry:${entry.id}:edit`);
    setError(null);
    try {
      await updateRegistry(entry.id, {
        name,
        executable,
        working_dir: workingDir,
        args: argsText.length === 0 ? [] : argsText.split("\n"),
      });
      await Promise.all([refreshRegistry(), refreshRuntime()]);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const unregister = async (entry: RegistryEntry) => {
    if (!window.confirm(`Unregister ${entry.name} (${entry.id})? Project files will not be deleted.`)) return;
    setPending(`registry:${entry.id}:unregister`);
    setError(null);
    try {
      await unregisterRegistry(entry.id);
      await Promise.all([refreshRegistry(), refreshDiscovery(), refreshRuntime()]);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const scan = async () => {
    setPending("discovery:scan");
    setError(null);
    try {
      setDiscovery(await scanDiscovery());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const register = async (candidate: DiscoveredProject) => {
    const executable = candidate.executable_candidates[0];
    if (!candidate.suggested_id || !executable || !candidate.runtime) return;
    const id = window.prompt("Stable MCP id", candidate.suggested_id);
    if (!id) return;
    const name = window.prompt("Display name", candidate.name);
    if (name === null) return;
    setPending(`discovery:${candidate.candidate_id}:register`);
    setError(null);
    try {
      await registerDiscovery(candidate.candidate_id, { id, name, executable });
      await Promise.all([refreshRegistry(), refreshDiscovery(), refreshRuntime()]);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending(null);
    }
  };

  const displayedUptime = selected?.state === "running" && selected.uptime_ms !== null
    ? selected.uptime_ms + uptimeTick * 1000
    : selected?.uptime_ms ?? null;
  const displayedTunnelUptime = tunnel?.state === "running" && tunnel.uptime_ms !== null
    ? tunnel.uptime_ms + uptimeTick * 1000
    : tunnel?.uptime_ms ?? null;

  const updateRuntimeStates = useMemo<Partial<Record<UpdateComponent, string>>>(() => ({
    filesystem: servers.filesystem?.state ?? "not registered",
    git: servers.git?.state ?? "not registered",
    exec: servers.exec?.state ?? "not registered",
    blender: servers.blender?.state ?? "not registered",
    gateway: tunnel ? `tunnel ${tunnel.state}` : "unknown",
    studio: connection === "connected" ? "serving dashboard" : connection,
    fleet: "control bundle",
    tunnel: tunnel?.state ?? "unknown",
  }), [servers, tunnel, connection]);

  return (
    <main className="shell">
      <header className="topbar">
        <div><p className="eyebrow">Local control plane</p><h1>MCP Studio</h1></div>
        <div className="connection-group"><span className="eyebrow">Studio realtime</span><span className={`connection connection-${connection}`}>{connection}</span></div>
      </header>

      {error && <div className="error-banner">{error}</div>}

      <section className="summary-grid" aria-label="Studio summary">
        <article className="summary-card"><span>MCP registered</span><strong>{registry.length}</strong></article>
        <article className="summary-card"><span>MCP running</span><strong>{running}</strong></article>
        <article className="summary-card"><span>MCP failed</span><strong>{failed}</strong></article>
        <article className="summary-card"><span>Tunnel</span><strong>{tunnel?.state ?? "loading"}</strong></article>
      </section>

      <UpdatesPanel
        inventory={updates}
        reconciliation={reconciliation}
        transactions={updateTransactions}
        runtimeStates={updateRuntimeStates}
        connection={connection}
        pending={pending}
        onCheckUpdates={runUpdateCheck}
        onPrepare={runPrepareUpdate}
        onApply={runApplyUpdate}
        onRefreshTransaction={runRefreshUpdateTransaction}
        onCheckReconciliation={runReconciliationCheck}
        onApplyReconciliation={runReconciliationApply}
      />

      <HistoryPanel refreshKey={historyRefreshKey} />

      <section className="detail-panel tunnel-panel" aria-label="Secure tunnel">
        <div className="detail-header"><div><p className="eyebrow">Secure tunnel</p><h2>{tunnel?.name ?? "Tunnel"}</h2></div>{tunnel && <span className={`state state-${tunnel.state}`}>{tunnel.state}</span>}</div>
        {tunnel && <>
          <dl className="metric-grid">
            <div><dt>Runtime</dt><dd>{tunnel.runtime_available ? "available" : "unavailable"}</dd></div><div><dt>PID</dt><dd>{tunnel.pid ?? "—"}</dd></div><div><dt>Uptime</dt><dd>{formatUptime(displayedTunnelUptime)}</dd></div><div><dt>Restarts</dt><dd>{tunnel.restart_count}</dd></div><div><dt>Crashes</dt><dd>{tunnel.crash_count}</dd></div><div><dt>Last exit</dt><dd>{tunnel.last_exit_code ?? "—"}</dd></div><div><dt>Last error</dt><dd>{tunnel.last_error ?? "—"}</dd></div>
          </dl>
          <div className="actions">{(["start", "restart", "stop"] as const).map((action) => <button key={action} disabled={!tunnelActionEnabled(tunnel, action) || pending !== null} onClick={() => void runTunnelAction(action)}>{pending === `tunnel:${action}` ? `${action}…` : action}</button>)}</div>
          <section className="logs"><div className="logs-header"><div><h3>Tunnel logs</h3><span>{tunnelLogs.length} recent</span></div></div><div className="log-viewer" ref={tunnelLogViewerRef}>{tunnelLogs.length === 0 && <p className="log-empty">No tunnel logs yet.</p>}{tunnelLogs.map((entry) => <div className={`log-line log-${entry.stream}`} key={entry.sequence}><time>{new Date(entry.timestamp_ms).toLocaleTimeString()}</time><span className="log-stream" title="Process stream, not severity">{entry.stream}</span><code>{formatLogMessage(entry.message)}</code></div>)}</div></section>
        </>}
      </section>

      <section className="workspace">
        <aside className="server-list"><h2>MCP servers</h2>{serverList.map((server) => <button className={`server-row ${server.id === selectedId ? "selected" : ""}`} key={server.id} onClick={() => setSelectedId(server.id)}><span><strong>{server.name}</strong><small>{server.id}</small></span><span className={`state state-${server.state}`}>{server.state}</span></button>)}</aside>
        <section className="detail-panel">
          {!selected ? <div className="empty-state">No MCP server selected.</div> : <>
            <div className="detail-header"><div><p className="eyebrow">MCP server · {selected.id}</p><h2>{selected.name}</h2></div><span className={`state state-${selected.state}`}>{selected.state}</span></div>
            <dl className="metric-grid"><div><dt>PID</dt><dd>{selected.pid ?? "—"}</dd></div><div><dt>Uptime</dt><dd>{formatUptime(displayedUptime)}</dd></div><div><dt>Restarts</dt><dd>{selected.restart_count}</dd></div><div><dt>Crashes</dt><dd>{selected.crash_count}</dd></div><div><dt>Last exit</dt><dd>{selected.last_exit_code ?? "—"}</dd></div><div><dt>Last error</dt><dd>{selected.last_error ?? "—"}</dd></div></dl>
            <div className="actions">{(["start", "restart", "stop"] as const).map((action) => <button key={action} disabled={!actionEnabled(selected, action) || pending !== null || registry.find((entry) => entry.id === selected.id)?.enabled === false} onClick={() => void runMcpAction(selected, action)}>{pending === `${selected.id}:${action}` ? `${action}…` : action}</button>)}</div>
            <section className="logs"><div className="logs-header"><div><h3>Recent MCP logs</h3><span>{visibleLogs.length} visible / {selectedLogs.length} total</span></div><div className="log-controls"><label>Stream<select value={logFilter} onChange={(event) => setLogFilter(event.target.value as LogFilter)}><option value="all">All</option><option value="stdout">stdout</option><option value="stderr">stderr</option><option value="studio">studio</option></select></label><label className="toggle-control"><input type="checkbox" checked={autoScroll} onChange={(event) => setAutoScroll(event.target.checked)} />Auto-scroll</label></div></div><div className="log-viewer" ref={logViewerRef}>{visibleLogs.length === 0 && <p className="log-empty">No logs in this view.</p>}{visibleLogs.map((entry) => <div className={`log-line log-${entry.stream}`} key={entry.sequence}><time>{new Date(entry.timestamp_ms).toLocaleTimeString()}</time><span className="log-stream">{entry.stream}</span><code>{formatLogMessage(entry.message)}</code></div>)}</div></section>
          </>}
        </section>
      </section>

      <section className="detail-panel" aria-label="MCP registry">
        <div className="detail-header"><div><p className="eyebrow">Persistent configuration</p><h2>Registry</h2></div><span>{registry.length} entries</span></div>
        <div className="registry-grid">
          {registry.map((entry) => {
            const runtime = servers[entry.id];
            const active = runtime && ["starting", "running", "stopping"].includes(runtime.state);
            return <article className="registry-card" key={entry.id}>
              <div className="detail-header"><div><strong>{entry.name}</strong><small>{entry.id} · {entry.runtime}</small></div><span className={`state ${entry.enabled ? "state-running" : "state-stopped"}`}>{entry.enabled ? "enabled" : "disabled"}</span></div>
              <dl className="metric-grid"><div><dt>Project</dt><dd>{entry.project_path}</dd></div><div><dt>Executable</dt><dd>{entry.executable}</dd></div><div><dt>Working dir</dt><dd>{entry.working_dir}</dd></div><div><dt>Runtime state</dt><dd>{runtime?.state ?? "stopped"}</dd></div></dl>
              <div className="actions"><button disabled={pending !== null || Boolean(active)} onClick={() => void editRegistry(entry)}>edit</button><button disabled={pending !== null || Boolean(active)} onClick={() => void toggleRegistry(entry)}>{entry.enabled ? "disable" : "enable"}</button><button disabled={pending !== null || Boolean(active)} onClick={() => void unregister(entry)}>unregister</button></div>
            </article>;
          })}
        </div>
      </section>

      <section className="detail-panel" aria-label="MCP discovery">
        <div className="detail-header"><div><p className="eyebrow">Metadata-only scan</p><h2>Discovery</h2></div><button disabled={pending !== null} onClick={() => void scan()}>{pending === "discovery:scan" ? "scanning…" : "scan"}</button></div>
        <div className="registry-grid">
          {discovery.map((candidate) => <article className="registry-card" key={candidate.candidate_id}>
            <div className="detail-header"><div><strong>{candidate.name}</strong><small>{candidate.project_path}</small></div><span>{candidate.runtime ?? "unsupported"}</span></div>
            <dl className="metric-grid"><div><dt>Manifest</dt><dd>{candidate.manifest ?? "—"}</dd></div><div><dt>Suggested ID</dt><dd>{candidate.suggested_id ?? "—"}</dd></div><div><dt>Executable</dt><dd>{candidate.executable_candidates[0] ?? "—"}</dd></div><div><dt>Status</dt><dd>{candidate.already_registered ? "registered" : "available"}</dd></div></dl>
            {candidate.warnings.length > 0 && <p className="log-empty">{candidate.warnings.join(" · ")}</p>}
            <div className="actions"><button disabled={pending !== null || candidate.already_registered || !candidate.runtime || !candidate.suggested_id || candidate.executable_candidates.length === 0} onClick={() => void register(candidate)}>review & register</button></div>
          </article>)}
        </div>
      </section>
    </main>
  );
}
