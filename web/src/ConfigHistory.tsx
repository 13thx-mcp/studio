import type { HistoricalConfigRevision } from "./history/types";

export default function ConfigHistory({
  revisions,
}: {
  revisions: HistoricalConfigRevision[];
}) {
  return (
    <section className="history-card" aria-label="Historical configuration">
      <div className="detail-header">
        <div>
          <p className="eyebrow">Historical observation</p>
          <h3>Configuration</h3>
        </div>
      </div>
      {revisions.length === 0 ? (
        <p className="log-empty">No configuration revisions recorded.</p>
      ) : (
        <div className="history-table-wrap">
          <table className="history-table">
            <thead>
              <tr>
                <th>Surface</th>
                <th>Observed</th>
                <th>Provenance</th>
                <th>Changes</th>
              </tr>
            </thead>
            <tbody>
              {revisions.map((revision) => (
                <tr key={revision.revision_id}>
                  <td>{revision.surface}</td>
                  <td>{new Date(revision.observed_at_ms).toLocaleString()}</td>
                  <td>{revision.provenance}</td>
                  <td>
                    {Array.isArray(revision.change_categories)
                      ? revision.change_categories.join(", ")
                      : "recorded"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <p className="history-note">
        Written/committed configuration is historical evidence; only a loaded revision proves
        what the current Studio process actually consumed.
      </p>
    </section>
  );
}
