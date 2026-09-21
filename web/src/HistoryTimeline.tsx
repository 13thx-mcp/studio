import type { HistoricalEvent, HistoryCoverage } from "./history/types";

export default function HistoryTimeline({
  events,
  coverage,
}: {
  events: HistoricalEvent[];
  coverage: HistoryCoverage | null;
}) {
  return (
    <section className="history-card" aria-label="Historical event timeline">
      <div className="detail-header">
        <div>
          <p className="eyebrow">Historical observation</p>
          <h3>Timeline</h3>
        </div>
        <span>{events.length} rows</span>
      </div>
      {coverage?.state === "partial" && (
        <p className="history-warning">Partial coverage · {coverage.reasons.join(", ") || "unknown gap"}</p>
      )}
      {events.length === 0 ? (
        <p className="log-empty">No observed history in this range.</p>
      ) : (
        <div className="history-table-wrap">
          <table className="history-table">
            <thead><tr><th>Seq</th><th>Observed</th><th>Event</th><th>Category</th><th>Evidence</th></tr></thead>
            <tbody>
              {events.map((event) => (
                <tr key={event.event_id}>
                  <td>{event.sequence}</td>
                  <td>{new Date(event.observed_at_ms).toLocaleString()}</td>
                  <td>{event.name}</td>
                  <td>{event.category}</td>
                  <td>{event.evidence_kind}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
