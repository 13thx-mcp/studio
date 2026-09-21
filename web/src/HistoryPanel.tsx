import { useEffect, useState } from "react";

import ConfigHistory from "./ConfigHistory";
import DriftHistory from "./DriftHistory";
import LineageHistory from "./LineageHistory";
import HistoryCharts from "./HistoryCharts";
import HistoryTimeline from "./HistoryTimeline";
import UpdateHistory from "./UpdateHistory";
import {
  getHistoryConfigRevisions,
  getHistoryDrift,
  getHistoryEvents,
  getHistoryLineage,
  getHistoryMetrics,
  getHistoryStatus,
  getHistorySubjects,
  getHistoryUpdates,
} from "./history/api";
import type {
  HistoricalConfigRevision,
  HistoricalDrift,
  HistoricalEvent,
  HistoricalLineage,
  HistoricalMetricTotal,
  HistoricalUpdate,
  HistoryCoverage,
  HistoryStatus,
} from "./history/types";

export interface HistoryViewState {
  status: HistoryStatus | null;
  events: HistoricalEvent[];
  updates: HistoricalUpdate[];
  drift: HistoricalDrift[];
  configRevisions: HistoricalConfigRevision[];
  lineage: HistoricalLineage[];
  totals: HistoricalMetricTotal[];
  coverage: HistoryCoverage | null;
  loading: boolean;
  error: string | null;
}

export function HistoryView({ state }: { state: HistoryViewState }) {
  return (
    <section className="detail-panel history-panel" aria-label="History">
      <div className="detail-header">
        <div><p className="eyebrow">Read-only persisted evidence</p><h2>History</h2></div>
        <span className={`state ${state.status?.state === "healthy" ? "state-running" : "state-failed"}`}>
          {state.status?.state ?? (state.loading ? "loading" : "unavailable")}
        </span>
      </div>
      {state.error && <p className="history-warning">{state.error === "history_expired" ? "History cursor expired; refresh from the current retained boundary." : state.error}</p>}
      {state.status?.coverage_state === "partial" && <p className="history-warning">Historical coverage is partial; missing evidence is not rendered as zero.</p>}
      <div className="history-grid">
        <HistoryTimeline events={state.events} coverage={state.coverage} />
        <HistoryCharts totals={state.totals} />
        <UpdateHistory updates={state.updates} />
        <DriftHistory drift={state.drift} />
        <ConfigHistory revisions={state.configRevisions} />
        <LineageHistory lineage={state.lineage} />
      </div>
    </section>
  );
}

export default function HistoryPanel({ refreshKey = 0 }: { refreshKey?: number }) {
  const [state, setState] = useState<HistoryViewState>({
    status: null,
    events: [],
    updates: [],
    drift: [],
    configRevisions: [],
    lineage: [],
    totals: [],
    coverage: null,
    loading: true,
    error: null,
  });

  useEffect(() => {
    const controller = new AbortController();
    void Promise.all([
      getHistoryStatus(controller.signal),
      getHistoryEvents(undefined, controller.signal),
      getHistoryUpdates(controller.signal),
      getHistoryDrift(controller.signal),
      getHistoryConfigRevisions(controller.signal),
      getHistorySubjects(controller.signal),
      getHistoryMetrics(controller.signal),
    ]).then(async ([status, events, updates, drift, configRevisions, subjects, metrics]) => {
      const lineage = (
        await Promise.all(
          subjects.items.slice(0, 20).map((subject) =>
            getHistoryLineage(subject.subject_id, controller.signal),
          ),
        )
      ).filter((item) => item.latest_observation_id !== null || item.boundary_reason !== "unknown_predecessor");
      setState({
        status,
        events: events.items,
        updates: updates.items,
        drift: drift.items,
        configRevisions: configRevisions.items,
        lineage,
        totals: metrics.totals,
        coverage: events.coverage,
        loading: false,
        error: null,
      });
    }).catch((cause) => {
      if (controller.signal.aborted) return;
      setState((current) => ({
        ...current,
        loading: false,
        error: cause instanceof Error ? cause.message : String(cause),
      }));
    });
    return () => controller.abort();
  }, [refreshKey]);

  return <HistoryView state={state} />;
}
