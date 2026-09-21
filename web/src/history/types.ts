export interface HistoryStatus {
  state: "healthy" | "degraded" | "unavailable" | string;
  reason_code: string | null;
  admission_available: boolean;
  pending_obligations: string;
  db_epoch: string | null;
  latest_committed_seq: string | null;
  metrics_through_seq: string | null;
  coverage_state: "complete" | "partial" | "unknown" | string;
  observed_at_ms: number;
}

export interface HistorySnapshot {
  db_epoch: string;
  through_seq: string;
  retention_epoch: string;
}

export interface HistoryCoverage {
  state: "complete" | "partial" | string;
  reasons: string[];
  since_ms: number | null;
}

export interface HistoryPage<T> {
  items: T[];
  next_cursor: string | null;
  snapshot: HistorySnapshot;
  coverage: HistoryCoverage;
}

export interface HistoricalEvent {
  sequence: string;
  event_id: string;
  name: string;
  category: string;
  subject_id: string;
  operation_id: string | null;
  observed_at_ms: number;
  source_at_ms: number | null;
  evidence_kind: string;
  time_quality: string;
  payload: Record<string, unknown> | null;
}

export interface HistoricalUpdate {
  history_id: string;
  transaction_id: string;
  component: string | null;
  source_version: string | null;
  target_version: string;
  last_phase: string;
  install_outcome: string;
  rollback_outcome: string;
  verification_scope: string;
  was_running: boolean | null;
  completeness: string;
  first_observed_sequence: string;
  terminal_sequence: string | null;
  actionable: false;
}

export interface HistoricalDrift {
  observation_id: string;
  state: string;
  phase: string;
  studio_config_activation: string;
  client_freshness: string;
  rollback_outcome: string;
  observation_kind: string;
  surfaces: string[];
  sequence: string;
}


export interface HistoricalConfigRevision {
  revision_id: string;
  surface: string;
  provenance: string;
  change_categories: unknown;
  previous_revision_id: string | null;
  boundary_reason: string | null;
  observed_at_ms: number;
  sequence: string;
}

export interface HistoricalSubject {
  subject_id: string;
  kind: string;
  component: string | null;
  retired_at_ms: number | null;
  as_of_sequence: string;
}

export interface HistoricalLineage {
  subject_id: string;
  latest_observation_id: string | null;
  previous_observation_id: string | null;
  last_verified_good_id: string | null;
  as_of_sequence: string;
  boundary_reason: string | null;
  current_match: "unknown" | string;
  actionable: false;
}

export interface HistoricalMetricTotal {
  subject_id: string;
  metric: string;
  count: string;
  sum: string;
  through_seq: string;
  coverage_start_ms: number;
  quality: "complete" | "partial" | "unknown";
}

export interface HistoricalMetricBucket {
  subject_id: string;
  metric: string;
  resolution_ms: number;
  bucket_start_ms: number;
  count: string;
  sum: string;
  min: string | null;
  max: string | null;
  through_seq: string;
  quality: "complete" | "partial" | "unknown";
}

export interface HistoricalMetrics {
  totals: HistoricalMetricTotal[];
  buckets: HistoricalMetricBucket[];
  through_seq: string;
}
