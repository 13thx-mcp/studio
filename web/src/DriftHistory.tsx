import type { HistoricalDrift } from "./history/types";

export default function DriftHistory({ drift }: { drift: HistoricalDrift[] }) {
  return (
    <section className="history-card" aria-label="Historical drift">
      <div className="detail-header"><div><p className="eyebrow">Historical observation</p><h3>Fleet drift</h3></div></div>
      {drift.length === 0 ? <p className="log-empty">No drift observations recorded.</p> : (
        <div className="history-table-wrap"><table className="history-table">
          <thead><tr><th>State</th><th>Phase</th><th>Kind</th><th>Surfaces</th></tr></thead>
          <tbody>{drift.map((item) => <tr key={item.observation_id}>
            <td>{item.state}</td><td>{item.phase}</td><td>{item.observation_kind}</td>
            <td>{item.surfaces.join(", ") || "none"}</td>
          </tr>)}</tbody>
        </table></div>
      )}
    </section>
  );
}
