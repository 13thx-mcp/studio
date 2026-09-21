import { metricValue } from "./history/state";
import type { HistoricalMetricTotal } from "./history/types";

export default function HistoryCharts({ totals }: { totals: HistoricalMetricTotal[] }) {
  const max = Math.max(1, ...totals.map((item) => metricValue(item.count)));
  return (
    <section className="history-card" aria-label="Historical metrics">
      <div className="detail-header"><div><p className="eyebrow">Recorded coverage</p><h3>Metrics</h3></div></div>
      {totals.length === 0 ? <p className="log-empty">Metrics are not aggregated yet.</p> : (
        <>
          <svg className="history-chart" role="img" aria-label="Historical metric counts" viewBox={`0 0 640 ${Math.max(80, totals.length * 28)}`}>
            {totals.map((item, index) => {
              const width = (metricValue(item.count) / max) * 360;
              return <g key={item.metric} transform={`translate(0 ${index * 28})`}>
                <text x="0" y="18">{item.metric}</text>
                <rect x="210" y="4" width={width} height="16" rx="3" />
                <text x={220 + width} y="18">{item.count}</text>
              </g>;
            })}
          </svg>
          <table className="history-table">
            <thead><tr><th>Metric</th><th>Count</th><th>Sum</th><th>Quality</th></tr></thead>
            <tbody>{totals.map((item) => <tr key={item.metric}><td>{item.metric}</td><td>{item.count}</td><td>{item.sum}</td><td>{item.quality}</td></tr>)}</tbody>
          </table>
        </>
      )}
    </section>
  );
}
