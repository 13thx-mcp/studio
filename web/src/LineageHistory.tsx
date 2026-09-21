import type { HistoricalLineage } from "./history/types";

export default function LineageHistory({
  lineage,
}: {
  lineage: HistoricalLineage[];
}) {
  return (
    <section className="history-card" aria-label="Historical artifact lineage">
      <div className="detail-header">
        <div>
          <p className="eyebrow">Observed lineage</p>
          <h3>Artifact lineage</h3>
        </div>
      </div>
      {lineage.length === 0 ? (
        <p className="log-empty">No verified install lineage recorded.</p>
      ) : (
        <div className="history-table-wrap">
          <table className="history-table">
            <thead>
              <tr>
                <th>Subject</th>
                <th>Current match</th>
                <th>Previous</th>
                <th>Boundary</th>
              </tr>
            </thead>
            <tbody>
              {lineage.map((item) => (
                <tr key={item.subject_id}>
                  <td>{item.subject_id}</td>
                  <td>{item.current_match}</td>
                  <td>{item.previous_observation_id ? "observed" : "unknown"}</td>
                  <td>{item.boundary_reason ?? "continuous observed coverage"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <p className="history-note">
        Lineage is observational and non-actionable. Unknown predecessor or retained boundaries are
        preserved instead of inferred.
      </p>
    </section>
  );
}
