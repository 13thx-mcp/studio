import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { HistoryView, type HistoryViewState } from "../HistoryPanel";
import { compareDecimalSequence, mergeHistoryEvents } from "./state";

const base: HistoryViewState = {
  status: {
    state: "healthy", reason_code: null, admission_available: true, pending_obligations: "0",
    db_epoch: "epoch", latest_committed_seq: "2", metrics_through_seq: "2",
    coverage_state: "complete", observed_at_ms: 1,
  },
  events: [],
  updates: [],
  drift: [],
  configRevisions: [],
  lineage: [],
  totals: [],
  coverage: { state: "complete", reasons: [], since_ms: null },
  loading: false,
  error: null,
};

describe("History UI", () => {
  it("handles empty, partial, expired and read-only states", () => {
    const html = renderToStaticMarkup(<HistoryView state={{
      ...base,
      status: { ...base.status!, coverage_state: "partial" },
      coverage: { state: "partial", reasons: ["interrupted"], since_ms: null },
      error: "history_expired",
    }} />);
    expect(html).toContain("History cursor expired");
    expect(html).toContain("Partial coverage");
    expect(html).toContain("No observed history");
    expect(html).toContain("read-only");
    expect(html).not.toContain(">apply<");
  });


  it("labels loaded configuration and unknown lineage without live authority", () => {
    const html = renderToStaticMarkup(<HistoryView state={{
      ...base,
      configRevisions: [{
        revision_id: "rev-1",
        surface: "studio.config",
        provenance: "loaded",
        change_categories: ["loaded"],
        previous_revision_id: null,
        boundary_reason: null,
        observed_at_ms: 1,
        sequence: "1",
      }],
      lineage: [{
        subject_id: "component-test",
        latest_observation_id: "obs-1",
        previous_observation_id: null,
        last_verified_good_id: "obs-1",
        as_of_sequence: "2",
        boundary_reason: "unknown_predecessor",
        current_match: "unknown",
        actionable: false,
      }],
    }} />);
    expect(html).toContain("only a loaded revision proves");
    expect(html).toContain("unknown_predecessor");
    expect(html).toContain("non-actionable");
    expect(html).not.toContain(">Apply<");
  });

  it("orders decimal sequences above JavaScript safe integer range", () => {
    expect(compareDecimalSequence("90071992547409930", "9007199254740993")).toBeGreaterThan(0);
  });

  it("deduplicates and caps retained history rows", () => {
    const event = (sequence: string) => ({
      sequence, event_id: `event-${sequence}`, name: "x", category: "observation",
      subject_id: "s", operation_id: null, observed_at_ms: 1, source_at_ms: null,
      evidence_kind: "owner", time_quality: "local", payload: null,
    });
    const rows = Array.from({ length: 1005 }, (_, index) => event(String(index + 1)));
    const merged = mergeHistoryEvents([event("1005")], rows);
    expect(merged).toHaveLength(1000);
    expect(merged[0].sequence).toBe("1005");
  });
});
