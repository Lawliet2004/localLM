import { useStateField, useIsStreaming } from "@openuidev/react-lang";
import * as C from "./richCatalog";
import type { Props } from "./componentTypes";
export function Chart({ props: original, domain, stateId }: Props<typeof C.Chart.props> & { domain?: [number, number]; stateId?: string }) {
  const selected = useStateField(stateId ?? `chart-${original.title}`, original.kind);
  const options = [
    "bar",
    "line",
    "scatter",
    ...(original.points.every((point) => point.value >= 0) &&
    original.points.some((point) => point.value > 0)
      ? ["pie", "donut"]
      : []),
  ];
  const p = {
    ...original,
    kind: options.includes(String(selected.value))
      ? String(selected.value)
      : original.kind,
  };
  const streaming = useIsStreaming();
  const max = Math.max(...p.points.map((v) => v.value), domain?.[1] ?? 1, 1),
    min = Math.min(...p.points.map((v) => v.value), domain?.[0] ?? 0, 0),
    range = max - min;
  const x = (i: number) => 40 + (i * 520) / Math.max(p.points.length - 1, 1),
    y = (v: number) => 210 - ((v - min) / range) * 180;
  const xs = p.points.map((point, i) => point.x ?? i);
  const xmin = Math.min(...xs), xmax = Math.max(...xs);
  const scatterX = (i: number) => xmin === xmax ? 300 : 40 + ((xs[i] - xmin) / (xmax - xmin)) * 520;
  let angle = -Math.PI / 2;
  const total = p.points.reduce((sum, d) => sum + d.value, 0);
  return (
    <figure className="genui-chart">
      <figcaption>{p.title}</figcaption>
      <label className="genui-control">
        Chart type{" "}
        <select
          value={p.kind}
          disabled={streaming}
          onChange={(e) =>
            selected.setValue(e.target.value as typeof original.kind)
          }
        >
          {options.map((kind) => (
            <option key={kind}>{kind}</option>
          ))}
        </select>
      </label>
      <svg
        viewBox="0 0 600 285"
        role="img"
        aria-label={`${p.kind} chart: ${p.title}`}
      >
        <title>{p.title}</title>
        {["pie", "donut"].includes(p.kind) ? (
          p.points.map((d, i) => {
            const start = angle;
            angle += (d.value / total) * Math.PI * 2;
            const end = angle;
            const large = end - start > Math.PI ? 1 : 0;
            const path =
              d.value === total
                ? "M 299 30 A 90 90 0 1 1 298.99 30 L 299 120 Z"
                : `M 299 120 L ${299 + 90 * Math.cos(start)} ${120 + 90 * Math.sin(start)} A 90 90 0 ${large} 1 ${299 + 90 * Math.cos(end)} ${120 + 90 * Math.sin(end)} Z`;
            return (
              <path key={i} className={`genui-series-${i % 5}`} d={path}>
                <title>
                  {d.label}: {d.value}
                </title>
              </path>
            );
          })
        ) : (
          <>
            <line
              x1="30"
              y1={y(0)}
              x2="580"
              y2={y(0)}
              className="genui-chart-axis"
            />
            <text x="4" y="24" className="genui-chart-label">
              {max}
            </text>
            <text x="4" y="220" className="genui-chart-label">
              {min}
            </text>
            {p.kind === "scatter" && (xmin === xmax ? [xmin] : [xmin, (xmin+xmax)/2, xmax]).map((tick,i) => <text key={i} x={xmin === xmax ? 300 : 40 + ((tick-xmin)/(xmax-xmin))*520} y="240" textAnchor="middle" className="genui-chart-label">{Number(tick.toPrecision(5))}</text>)}
            {p.kind !== "scatter" && p.points.length <= 12 &&
              p.points.map((d, i) => (
                <text
                  key={i}
                  x={
                    p.kind === "bar"
                      ? 35 + (i * 540) / p.points.length + 250 / p.points.length
                      : p.kind === "scatter" ? scatterX(i) : x(i)
                  }
                  y="240"
                  textAnchor="middle"
                  className="genui-chart-label"
                >
                  {p.kind === "scatter" ? xs[i] : d.label.slice(0, 12)}
                </text>
              ))}
            {p.kind === "line" && (
              <polyline
                points={p.points
                  .map((d, i) => `${x(i)},${y(d.value)}`)
                  .join(" ")}
                fill="none"
                className="genui-chart-line"
              />
            )}
            {p.points.map((d, i) =>
              p.kind === "bar" ? (
                <rect
                  key={i}
                  x={35 + (i * 540) / p.points.length}
                  y={Math.min(y(d.value), y(0))}
                  width={Math.max(1, 500 / p.points.length)}
                  height={Math.abs(y(d.value) - y(0))}
                  className={`genui-series-${i % 5}`}
                >
                  <title>
                    {d.label}: {d.value}
                  </title>
                </rect>
              ) : (
                <circle
                  key={i}
                  cx={p.kind === "scatter" ? scatterX(i) : x(i)}
                  cy={y(d.value)}
                  r="4"
                  className={`genui-series-${i % 5}`}
                >
                  <title>
                    {d.label}: {p.kind === "scatter" ? `x=${xs[i]}, y=` : ""}{d.value}
                  </title>
                </circle>
              ),
            )}
          </>
        )}
        {p.xLabel && <text x="300" y="280" textAnchor="middle" className="genui-chart-label">{p.xLabel}</text>}
        {p.yLabel && <text x="40" y="15" className="genui-chart-label">{p.yLabel}</text>}
        {p.kind === "donut" && (
          <circle cx="299" cy="120" r="48" className="genui-chart-hole" />
        )}
      </svg>
      <details>
        <summary>Chart values and legend</summary>
        <table className="genui-table">
          <thead>
            <tr>
              <th>Label</th>
              <th>{p.kind === "scatter" ? "Y" : "Value"}{p.yLabel ? ` (${p.yLabel})` : ""}</th>
              {p.kind === "scatter" && <th>X{p.xLabel ? ` (${p.xLabel})` : ""}</th>}
            </tr>
          </thead>
          <tbody>
            {p.points.map((d, i) => (
              <tr key={i}>
                <th>
                  <span
                    className={`genui-legend genui-series-${i % 5}`}
                    aria-hidden="true"
                  />
                  {d.label}
                </th>
                <td>{d.value}</td>
                {p.kind === "scatter" && <td>{xs[i]}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}
