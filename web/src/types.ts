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

export type RuntimeKind = "rust" | "node" | "python";

export interface RegistryEntry {
  id: string;
  name: string;
  enabled: boolean;
  runtime: RuntimeKind;
  project_path: string;
  executable: string;
  working_dir: string;
  args: string[];
}

export interface RegistryUpdate {
  name: string;
  executable: string;
  working_dir: string;
  args: string[];
}

export interface DiscoveredProject {
  candidate_id: string;
  suggested_id: string | null;
  name: string;
  project_path: string;
  runtime: RuntimeKind | null;
  manifest: string | null;
  executable_candidates: string[];
  warnings: string[];
  already_registered: boolean;
}

export interface RegisterDiscoveryRequest {
  id: string;
  name?: string;
  executable: string;
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
  | { type: "registry_changed" }
  | { type: "discovery_changed" }
  | { type: "updates_changed" }
  | { type: "update_transaction"; transaction: UpdateTransaction }
  | { type: "reconciliation_changed"; status: ReconciliationView }
  | {
      type: "history_committed";
      db_epoch: string;
      latest_seq: string;
      retention_epoch: string;
    }
  | {
      type: "history_health";
      state: string;
      reason_code: string | null;
      admission_available: boolean;
    }
  | { type: "resync_required" };

export type UpdateComponent =
  | "filesystem"
  | "git"
  | "exec"
  | "gateway"
  | "blender"
  | "studio"
  | "fleet"
  | "tunnel";

export type ComponentClass = "mcp_binary" | "service" | "control_bundle" | "upstream_runtime";
export type ReleaseProviderId = "github_13thx" | "github_openai";
export type HostMode = "source_present" | "runtime_only";
export type InventoryHealth = "healthy" | "broken" | "unknown";
export type DriftState =
  | "current"
  | "update_available"
  | "installed_restart_required"
  | "drifted"
  | "unknown"
  | "broken";
export type CheckStatus = "never" | "ok" | "error";

export interface Platform {
  os: "darwin";
  arch: "amd64" | "arm64";
}

export interface ReleaseCheckView {
  checked_at_ms: number | null;
  status: CheckStatus;
  error: string | null;
}

export interface UpdateInventory {
  component: UpdateComponent;
  display_name: string;
  class: ComponentClass;
  provider: ReleaseProviderId;
  platform: Platform;
  host_mode: HostMode;
  source_present: boolean;
  installed_version: string | null;
  running_version: string | null;
  desired_version: string | null;
  latest_version: string | null;
  update_available: boolean;
  installation_health: InventoryHealth;
  drift: DriftState;
  last_check: ReleaseCheckView;
}

export type UpdateTransactionPhase =
  | "preparing"
  | "staged"
  | "stopping"
  | "activating"
  | "starting"
  | "verifying"
  | "gateway_stopping"
  | "gateway_restarting"
  | "gateway_reconnecting"
  | "gateway_catalog_verifying"
  | "activation_pending"
  | "external_activating"
  | "health_verifying"
  | "rolling_back"
  | "rolled_back"
  | "completed"
  | "failed"
  | "rollback_failed";

export interface UpdateTransaction {
  transaction_id: string;
  component: UpdateComponent;
  source_version: string | null;
  target_version: string;
  phase: UpdateTransactionPhase;
  was_running: boolean | null;
  rollback_succeeded: boolean | null;
  error: string | null;
  updated_at_ms: number;
}

export type ReconciliationState =
  | "unknown"
  | "synchronized"
  | "managed_safe_drift"
  | "unmanaged_conflict"
  | "broken";

export type ReconciliationPhase =
  | "idle"
  | "checking"
  | "reconcile_ready"
  | "snapshotting"
  | "rendering"
  | "validating"
  | "reloading_gateway"
  | "restarting_tunnel"
  | "catalog_verifying"
  | "synchronized"
  | "failed"
  | "rollback_failed";

export type ClientFreshness = "unknown" | "refresh_pending";
export type StudioConfigActivation = "unknown" | "active" | "restart_required";

export interface ReconciliationView {
  host_id: string | null;
  state: ReconciliationState;
  phase: ReconciliationPhase;
  safe_to_reconcile: boolean;
  affected_surfaces: string[];
  gateway_reload_required: boolean;
  tunnel_restart_required: boolean;
  studio_restart_required: boolean;
  studio_config_activation: StudioConfigActivation;
  catalog_fingerprint: string | null;
  generation: number;
  client_freshness: ClientFreshness;
  rollback_succeeded: boolean | null;
  last_error: string | null;
  checked_at_ms: number;
}
