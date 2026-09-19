import type {
  DriftState,
  UpdateComponent,
  UpdateInventory,
  UpdateTransaction,
  UpdateTransactionPhase,
} from "./types";

export const PENDING_UPDATE_STORAGE_KEY = "mcp-studio.pending-update-transactions";

export interface PendingUpdateRef {
  transaction_id: string;
  component: UpdateComponent;
}

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const updateCapable = new Set<UpdateComponent>([
  "filesystem",
  "git",
  "exec",
  "gateway",
  "blender",
  "studio",
  "fleet",
  "tunnel",
]);

const phaseLabels: Record<UpdateTransactionPhase, string> = {
  preparing: "Preparing release",
  staged: "Staged and verified",
  stopping: "Stopping component",
  activating: "Activating release",
  starting: "Starting component",
  verifying: "Verifying health",
  gateway_stopping: "Stopping Gateway owner",
  gateway_restarting: "Restarting Gateway",
  gateway_reconnecting: "Reconnecting control path",
  gateway_catalog_verifying: "Verifying Gateway catalog",
  activation_pending: "Activation pending",
  external_activating: "External launcher activating",
  health_verifying: "Verifying replacement health",
  rolling_back: "Rolling back",
  rolled_back: "Rolled back",
  completed: "Complete",
  failed: "Failed",
  rollback_failed: "Rollback failed",
};

const errorLabels: Record<string, string> = {
  release_provider_unreachable: "Release provider is unreachable.",
  release_unavailable: "Requested release is unavailable.",
  unsupported_platform: "This platform is not supported by the release.",
  artifact_download_failed: "Release download failed.",
  checksum_mismatch: "Release checksum verification failed.",
  staging_failed: "Release package validation or staging failed.",
  installed_identity_error: "Installed runtime identity could not be verified.",
  update_transaction_failed: "Update transaction could not proceed.",
  update_activation_failed: "Release activation failed.",
  update_verification_failed: "Replacement health or identity verification failed.",
  rollback_failed: "Rollback failed and operator repair is required.",
  launcher_unavailable: "The external Studio activation launcher is unavailable.",
};

export function supportsManualUpdate(component: UpdateComponent): boolean {
  return updateCapable.has(component);
}

export function phaseLabel(phase: UpdateTransactionPhase): string {
  return phaseLabels[phase] ?? "Unknown update state";
}

export function isTerminalTransaction(phase: UpdateTransactionPhase): boolean {
  return ["completed", "rolled_back", "failed", "rollback_failed"].includes(phase);
}

export function isExpectedReconnect(transaction: UpdateTransaction): boolean {
  if (transaction.component === "studio") {
    return ["activation_pending", "external_activating", "health_verifying"].includes(
      transaction.phase,
    );
  }
  if (transaction.component === "gateway") {
    return [
      "gateway_stopping",
      "gateway_restarting",
      "gateway_reconnecting",
      "gateway_catalog_verifying",
    ].includes(transaction.phase);
  }
  return false;
}

export function driftLabel(drift: DriftState): string {
  switch (drift) {
    case "current":
      return "Current";
    case "update_available":
      return "Update available";
    case "installed_restart_required":
      return "Restart required";
    case "drifted":
      return "Version drift";
    case "broken":
      return "Broken";
    case "unknown":
      return "Unknown";
  }
}

export function providerLabel(provider: UpdateInventory["provider"]): string {
  return provider === "github_openai" ? "OpenAI GitHub Releases" : "13thx GitHub Releases";
}

export function platformLabel(inventory: UpdateInventory): string {
  return `${inventory.platform.os}-${inventory.platform.arch}`;
}

export function componentImpact(
  component: UpdateComponent,
  runningIdentityKnown: boolean,
): string {
  switch (component) {
    case "gateway":
      return "Gateway update may reconnect the tunnel-owned control path and interrupt this dashboard briefly.";
    case "studio":
      return "Studio update hands activation to the external Fleet launcher and this dashboard is expected to reconnect.";
    case "fleet":
      return "Fleet is a control bundle; no managed process is started as part of activation.";
    case "tunnel":
      return runningIdentityKnown
        ? "Tunnel update switches the verified versioned runtime, restarts only when currently running, and preserves config/credentials."
        : "Tunnel update preserves stopped/running ownership from the backend lifecycle state, switches only the versioned runtime, and preserves config/credentials.";
    default:
      return runningIdentityKnown
        ? "A running MCP may be stopped, replaced, restarted, and health-verified. A stopped MCP remains stopped."
        : "Runtime running identity is not currently proven; backend lifecycle rules remain authoritative.";
  }
}

export function rollbackMessage(component: UpdateComponent): string {
  if (!supportsManualUpdate(component)) return "No activation transaction is available.";
  return "Automatic rollback is prepared/retained by the backend when destructive activation requires it.";
}

export function targetVersionFor(inventory: UpdateInventory): string {
  return inventory.latest_version ?? inventory.desired_version ?? inventory.installed_version ?? "";
}

export function canPrepareUpdate(inventory: UpdateInventory, targetVersion: string): boolean {
  return (
    supportsManualUpdate(inventory.component) &&
    inventory.installation_health !== "broken" &&
    targetVersion.trim().length > 0 &&
    (targetVersion.trim() !== inventory.installed_version || inventory.component === "tunnel")
  );
}

export function prepareConfirmation(
  inventory: UpdateInventory,
  targetVersion: string,
  runtimeState: string,
): string {
  const forceReinstall =
    inventory.component === "tunnel" &&
    inventory.installed_version !== null &&
    targetVersion === inventory.installed_version;
  return [
    `Prepare ${inventory.display_name} update?`,
    forceReinstall ? "Mode: verified same-version force reinstall" : "",
    `Installed: ${inventory.installed_version ?? "unknown"}`,
    `Running: ${inventory.running_version ?? "unknown"}`,
    `Target: ${targetVersion || "not selected"}`,
    `Runtime state: ${runtimeState}`,
    "",
    componentImpact(inventory.component, inventory.running_version !== null),
    rollbackMessage(inventory.component),
  ].join("\n");
}

export function applyConfirmation(
  inventory: UpdateInventory,
  transaction: UpdateTransaction,
  runtimeState: string,
): string {
  const reconnect = ["gateway", "studio"].includes(inventory.component)
    ? "\n\nThis operation may interrupt the dashboard. The transaction ID will be retained and queried after reconnect."
    : "";
  return [
    `Apply staged ${inventory.display_name} update?`,
    "",
    `Current: ${transaction.source_version ?? inventory.installed_version ?? "unknown"}`,
    `Target: ${transaction.target_version}`,
    `Runtime state: ${runtimeState}`,
    "",
    componentImpact(
      inventory.component,
      transaction.was_running ?? inventory.running_version !== null,
    ),
    rollbackMessage(inventory.component),
  ].join("\n") + reconnect;
}

export function transactionResult(transaction: UpdateTransaction): string {
  if (transaction.phase === "completed") return "Updated successfully";
  if (transaction.phase === "rolled_back") return "Update failed; rollback succeeded";
  if (transaction.phase === "rollback_failed") return "Rollback failed";
  if (transaction.phase === "failed") {
    return transaction.rollback_succeeded
      ? "Update failed; rollback succeeded"
      : "Update failed";
  }
  return phaseLabel(transaction.phase);
}

export function actionableUpdateError(error: string | null): string | null {
  if (!error) return null;
  return errorLabels[error] ?? error.replaceAll("_", " ");
}

export function persistPendingTransaction(storage: StorageLike, transaction: UpdateTransaction): void {
  const current = readPendingTransactions(storage).filter(
    (item) => item.transaction_id !== transaction.transaction_id,
  );
  if (!isTerminalTransaction(transaction.phase)) {
    current.push({
      transaction_id: transaction.transaction_id,
      component: transaction.component,
    });
  }
  writePendingTransactions(storage, current);
}

export function readPendingTransactions(storage: StorageLike): PendingUpdateRef[] {
  const raw = storage.getItem(PENDING_UPDATE_STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    const candidates = Array.isArray(parsed) ? parsed : [parsed];
    const valid: PendingUpdateRef[] = [];
    for (const candidate of candidates) {
      if (!candidate || typeof candidate !== "object") continue;
      const value = candidate as Partial<PendingUpdateRef>;
      if (
        typeof value.transaction_id !== "string" ||
        typeof value.component !== "string" ||
        !value.transaction_id.startsWith("txn-")
      ) {
        continue;
      }
      const component = value.component as UpdateComponent;
      if (
        ![
          "filesystem",
          "git",
          "exec",
          "gateway",
          "blender",
          "studio",
          "fleet",
          "tunnel",
        ].includes(component)
      ) {
        continue;
      }
      if (!valid.some((item) => item.transaction_id === value.transaction_id)) {
        valid.push({ transaction_id: value.transaction_id, component });
      }
    }
    return valid;
  } catch {
    return [];
  }
}

export function readPendingTransaction(storage: StorageLike): PendingUpdateRef | null {
  return readPendingTransactions(storage)[0] ?? null;
}

export function clearPendingTransaction(storage: StorageLike, transactionId?: string): void {
  if (!transactionId) {
    storage.removeItem(PENDING_UPDATE_STORAGE_KEY);
    return;
  }
  writePendingTransactions(
    storage,
    readPendingTransactions(storage).filter(
      (item) => item.transaction_id !== transactionId,
    ),
  );
}

function writePendingTransactions(storage: StorageLike, values: PendingUpdateRef[]): void {
  if (values.length === 0) {
    storage.removeItem(PENDING_UPDATE_STORAGE_KEY);
  } else {
    storage.setItem(PENDING_UPDATE_STORAGE_KEY, JSON.stringify(values));
  }
}

export function updateCardStatus(inventory: UpdateInventory): {
  label: string;
  attention: boolean;
  danger: boolean;
} {
  return {
    label: driftLabel(inventory.drift),
    attention: ["update_available", "installed_restart_required", "drifted"].includes(
      inventory.drift,
    ),
    danger: inventory.drift === "broken",
  };
}