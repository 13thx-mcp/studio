import type { HistoricalUpdate } from "./history/types";

export default function UpdateHistory({ updates }: { updates: HistoricalUpdate[] }) {
  return (
    <section className="history-card" aria-label="Historical updates">
      <div className="detail-header"><div><p className="eyebrow">Historical observation</p><h3>Updates</h3></div></div>
      {updates.length === 0 ? <p className="log-empty">No update attempts recorded.</p> : (
        <div className="history-table-wrap"><table className="history-table">
          <thead><tr><th>Component</th><th>Target</th><th>Outcome</th><th>Rollback</th><th>Proof</th></tr></thead>
          <tbody>{updates.map((item) => <tr key={item.history_id}>
            <td>{item.component ?? "unknown"}</td><td>{item.target_version}</td>
            <td>{item.install_outcome}</td><td>{item.rollback_outcome}</td>
            <td>{item.verification_scope}</td>
          </tr>)}</tbody>
        </table></div>
      )}
      <p className="history-note">Historical rows are read-only and never authorize apply/resume actions.</p>
    </section>
  );
}
