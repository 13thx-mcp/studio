import type {
  DiscoveredProject,
  LogEntry,
  ProcessStatus,
  RegisterDiscoveryRequest,
  RegistryEntry,
  RegistryUpdate,
  TunnelLogEntry,
  TunnelStatus,
  UpdateComponent,
  UpdateInventory,
  UpdateTransaction,
  ReconciliationView,
} from "./types";

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

export function getLogs(id: string): Promise<LogEntry[]> {
  return fetch(`/api/mcp/${encodeURIComponent(id)}/logs`).then(readJson<LogEntry[]>);
}

export function lifecycleAction(
  id: string,
  action: "start" | "stop" | "restart",
): Promise<ProcessStatus> {
  return fetch(`/api/mcp/${encodeURIComponent(id)}/${action}`, { method: "POST" }).then(
    readJson<ProcessStatus>,
  );
}

export function listRegistry(): Promise<RegistryEntry[]> {
  return fetch("/api/registry").then(readJson<RegistryEntry[]>);
}

export function updateRegistry(id: string, update: RegistryUpdate): Promise<RegistryEntry> {
  return fetch(`/api/registry/${encodeURIComponent(id)}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(update),
  }).then(readJson<RegistryEntry>);
}

export function setRegistryEnabled(id: string, enabled: boolean): Promise<RegistryEntry> {
  return fetch(`/api/registry/${encodeURIComponent(id)}/${enabled ? "enable" : "disable"}`, {
    method: "POST",
  }).then(readJson<RegistryEntry>);
}

export function unregisterRegistry(id: string): Promise<RegistryEntry> {
  return fetch(`/api/registry/${encodeURIComponent(id)}`, { method: "DELETE" }).then(
    readJson<RegistryEntry>,
  );
}

export function listDiscovery(): Promise<DiscoveredProject[]> {
  return fetch("/api/discovery").then(readJson<DiscoveredProject[]>);
}

export function scanDiscovery(): Promise<DiscoveredProject[]> {
  return fetch("/api/discovery/scan", { method: "POST" }).then(readJson<DiscoveredProject[]>);
}

export function registerDiscovery(
  candidateId: string,
  request: RegisterDiscoveryRequest,
): Promise<RegistryEntry> {
  return fetch(`/api/discovery/${encodeURIComponent(candidateId)}/register`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(request),
  }).then(readJson<RegistryEntry>);
}

export function getTunnel(): Promise<TunnelStatus> {
  return fetch("/api/tunnel").then(readJson<TunnelStatus>);
}

export function getTunnelLogs(): Promise<TunnelLogEntry[]> {
  return fetch("/api/tunnel/logs").then(readJson<TunnelLogEntry[]>);
}

export function tunnelLifecycleAction(
  action: "start" | "stop" | "restart",
): Promise<TunnelStatus> {
  return fetch(`/api/tunnel/${action}`, { method: "POST" }).then(readJson<TunnelStatus>);
}

export function listUpdates(): Promise<UpdateInventory[]> {
  return fetch("/api/updates").then(readJson<UpdateInventory[]>);
}

export function checkUpdates(): Promise<UpdateInventory[]> {
  return fetch("/api/updates/check", { method: "POST" }).then(readJson<UpdateInventory[]>);
}

export function prepareUpdate(
  component: UpdateComponent,
  version: string,
): Promise<UpdateTransaction> {
  return fetch(`/api/updates/${encodeURIComponent(component)}/prepare`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ version }),
  }).then(readJson<UpdateTransaction>);
}

export function applyUpdate(
  component: UpdateComponent,
  transactionId: string,
): Promise<UpdateTransaction> {
  return fetch(`/api/updates/${encodeURIComponent(component)}/apply`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ transaction_id: transactionId }),
  }).then(readJson<UpdateTransaction>);
}

export function getUpdateTransaction(transactionId: string): Promise<UpdateTransaction> {
  return fetch(`/api/update-transactions/${encodeURIComponent(transactionId)}`).then(
    readJson<UpdateTransaction>,
  );
}

export function getReconciliation(): Promise<ReconciliationView> {
  return fetch("/api/reconciliation").then(readJson<ReconciliationView>);
}

export function checkReconciliation(): Promise<ReconciliationView> {
  return fetch("/api/reconciliation/check", { method: "POST" }).then(
    readJson<ReconciliationView>,
  );
}

export function applyReconciliation(): Promise<ReconciliationView> {
  return fetch("/api/reconciliation/apply", { method: "POST" }).then(
    readJson<ReconciliationView>,
  );
}
