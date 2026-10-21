import * as C from "./richCatalog";
import type { Props } from "./componentTypes";
import { useMemo, useState, useEffect, useId } from "react";
import { useStateField, useIsStreaming, useGetFieldValue, useSetDefaultValue, useFormName } from "@openuidev/react-lang";
import {
  binarySearch,
  lru,
  sorting,
  traversal,
  sqlJoin,
  type SimulationFrame,
} from "./simulations";
export function Quiz({ props: p }: Props<typeof C.Quiz.props>) {
  const instance = useId();
  const field = useStateField<Record<string, number>>(p.id, {});
  const answers =
    field.value && typeof field.value === "object" ? field.value : {};
  const streaming = useIsStreaming();
  return (
    <fieldset className="genui-exercise" disabled={streaming}>
      <legend>{p.title}</legend>
      {p.questions.map((q, i) => (
        <fieldset key={i}>
          <legend>{q.question}</legend>
          {q.options.map((option, j) => (
            <label key={j} className="genui-choice">
              <input
                type="radio"
                name={`${instance}-${p.id}-${i}`}
                checked={answers[i] === j}
                onChange={() => field.setValue({ ...answers, [i]: j })}
              />
              {option}
            </label>
          ))}
          {answers[i] !== undefined && (
            <p role="status">
              {answers[i] === q.answer ? "Correct." : "Try again."}{" "}
              {q.explanation}
            </p>
          )}
        </fieldset>
      ))}
      <output>
        Score: {p.questions.filter((q, i) => answers[i] === q.answer).length} /{" "}
        {p.questions.length}
      </output>
    </fieldset>
  );
}
export function Flashcard({ props: p }: Props<typeof C.Flashcard.props>) {
  const state = useStateField(p.id, false);
  return (
    <div className="genui-exercise">
      <p>{state.value ? p.back : p.front}</p>
      <button
        type="button"
        className="genui-btn"
        disabled={useIsStreaming()}
        onClick={() => state.setValue(!state.value)}
        aria-pressed={!!state.value}
      >
        {state.value ? "Show question" : "Show answer"}
      </button>
    </div>
  );
}
export function Example({
  props: p,
}: Props<typeof C.InteractiveExample.props>) {
  const step = useStateField(`${p.id}-step`, 0),
    rate = useStateField(`${p.id}-rate`, 0.1),
    join = useStateField(`${p.id}-join`, "inner");
  const streaming = useIsStreaming();
  const frames = useMemo<SimulationFrame[]>(() => {
    switch (p.kind) {
      case "sorting":
        return sorting(p.values);
      case "binary-search":
        return binarySearch(p.values, p.target ?? p.values[0]);
      case "lru":
        return lru(p.values, p.capacity ?? 3);
      case "bfs":
      case "dfs":
        return traversal(
          p.values,
          p.edges ??
            p.values.slice(1).map((_, i) => [i, i + 1] as [number, number]),
          p.kind === "dfs",
        );
      case "gradient-descent": {
        let weight = p.values[0];
        return Array.from({ length: 31 }, (_, i) => {
          const result = {
            values: [weight],
            active: [0],
            message: `Epoch ${i}: w=${weight.toFixed(4)}, loss=${(weight * weight).toFixed(4)}`,
          };
          weight -= 2 * Number(rate.value) * weight;
          return result;
        });
      }
      default:
        return [
          {
            values: p.values,
            active: [],
            message: "Select a join type below.",
          },
        ];
    }
  }, [p.kind, p.values, p.target, p.capacity, p.edges, rate.value]);
  const position = Math.max(
    0,
    Math.min(frames.length - 1, Number(step.value) || 0),
  );
  const frame = frames[position];
  const left = p.values.slice(0, Math.ceil(p.values.length / 2)),
    right = p.values.slice(Math.ceil(p.values.length / 2));
  const rows = sqlJoin(
    left,
    right,
    join.value === "left" || join.value === "full" ? join.value : "inner",
  ).map((row) => row.map((v) => (v === null ? "NULL" : v)).join(" | "));
  const [playing, setPlaying] = useState(false);
  const speed = useStateField(`${p.id}-speed`, 700);
  useEffect(() => {
    if (!playing || streaming) return;
    if (position >= frames.length - 1) {
      setPlaying(false);
      return;
    }
    const timer = setTimeout(
      () => step.setValue(position + 1),
      Math.max(100, Math.min(2000, Number(speed.value) || 700)),
    );
    return () => clearTimeout(timer);
  }, [playing, streaming, position, frames.length, speed.value, step.setValue]);
  return (
    <section className="genui-exercise" aria-label={`${p.kind} simulator`}>
      <h3>{p.kind.replace(/-/g, " ")}</h3>
      <p className="genui-muted">
        Deterministic educational example
        {p.kind === "binary-search" ? " · input sorted ascending" : ""}; no code
        or model training is executed.
      </p>
      {p.kind === "gradient-descent" && (
        <label>
          Learning rate{" "}
          <input
            type="range"
            min="0.01"
            max="0.5"
            step="0.01"
            value={Number(rate.value) || 0.1}
            disabled={streaming}
            onChange={(e) => {
              rate.setValue(e.target.valueAsNumber);
              step.setValue(0);
            }}
          />
          <output>{Number(rate.value).toFixed(2)}</output>
        </label>
      )}
      {p.kind === "sql-join" ? (
        <>
          <label>
            Join{" "}
            <select
              value={String(join.value)}
              disabled={streaming}
              onChange={(e) => join.setValue(e.target.value)}
            >
              <option value="inner">Inner</option>
              <option value="left">Left</option>
              <option value="full">Full outer</option>
            </select>
          </label>
          <p>
            Left: {left.join(", ")} · Right: {right.join(", ")}
          </p>
          <pre>{rows.join("\n") || "No matching rows"}</pre>
        </>
      ) : (
        <>
          <div className="genui-cells">
            {frame?.values.map((value, i) => (
              <span
                className={frame.active.includes(i) ? "is-active" : ""}
                key={i}
              >
                {Number.isInteger(value) ? value : value.toFixed(3)}
              </span>
            ))}
          </div>
          <p role="status">{frame?.message}</p>
          <div className="genui-controls">
            <button
              type="button"
              className="genui-btn"
              disabled={streaming || position >= frames.length - 1}
              aria-pressed={playing}
              onClick={() => setPlaying((v) => !v)}
            >
              {playing ? "Pause" : "Play"}
            </button>
            <label>
              Step delay{" "}
              <input
                type="range"
                min="100"
                max="2000"
                step="100"
                disabled={streaming}
                value={Number(speed.value) || 700}
                onChange={(e) => speed.setValue(e.target.valueAsNumber)}
              />
            </label>
            <button
              type="button"
              className="genui-btn"
              disabled={streaming || position === 0}
              onClick={() => step.setValue(position - 1)}
            >
              Previous
            </button>
            <button
              type="button"
              className="genui-btn"
              disabled={streaming || position === frames.length - 1}
              onClick={() => step.setValue(position + 1)}
            >
              Next step
            </button>
            <button
              type="button"
              className="genui-btn"
              disabled={streaming}
              onClick={() => step.setValue(0)}
            >
              Reset
            </button>
            <output>
              Step {position + 1} / {frames.length}
            </output>
          </div>
        </>
      )}
    </section>
  );
}
export function Control({
  kind,
  props: p,
}: {
  kind: "Slider" | "Select" | "Toggle" | "Input";
  props: Record<string, unknown>;
}) {
  const field = useStateField(String(p.id), p.value),
    streaming = useIsStreaming();
  const formName = useFormName(), getField = useGetFieldValue();
  useSetDefaultValue({formName,componentType:kind,name:String(p.id),existingValue:getField(formName,String(p.id)),defaultValue:p.value});
  return (
    <label className="genui-control">
      <span>{String(p.label)}</span>
      {kind === "Select" ? (
        <select
          disabled={streaming}
          value={String(field.value)}
          onChange={(e) => field.setValue(e.target.value)}
        >
          {(p.options as string[]).map((v, i) => (
            <option key={i}>{v}</option>
          ))}
        </select>
      ) : kind === "Toggle" ? (
        <input
          disabled={streaming}
          type="checkbox"
          checked={!!field.value}
          onChange={(e) => field.setValue(e.target.checked)}
        />
      ) : (
        <input
          aria-label={String(p.label)}
          disabled={streaming}
          type={kind === "Slider" ? "range" : "text"}
          min={Number(p.min)}
          max={Number(p.max)}
          step={Number(p.step) || 1}
          maxLength={2000}
          value={String(field.value ?? "")}
          onChange={(e) =>
            field.setValue(
              kind === "Slider" ? e.target.valueAsNumber : e.target.value,
            )
          }
        />
      )}
      <output>{String(field.value ?? "")}</output>
    </label>
  );
}

export function Calculator({props:p}:Props<typeof C.Calculator.props>){
 const a=useStateField<number|string>(`${p.id}-a`,p.a),b=useStateField<number|string>(`${p.id}-b`,p.b);const streaming=useIsStreaming();
 const first=a.value === "" ? NaN : Number(a.value),second=b.value === "" ? NaN : Number(b.value);
 const operandsValid=Number.isFinite(first)&&Number.isFinite(second)&&Math.abs(first)<=1e12&&Math.abs(second)<=1e12;
 const value=p.operation==='add'?first+second:p.operation==='multiply'?first*second:p.operation==='divide'?first/second:Math.pow(first,second);
 return <fieldset className="genui-exercise" disabled={streaming}><legend>{p.title}</legend><label className="genui-control">First value <input type="number" min="-1000000000000" max="1000000000000" value={String(a.value)} onChange={e=>a.setValue(e.target.value)}/></label><label className="genui-control">Second value <input type="number" min="-1000000000000" max="1000000000000" value={String(b.value)} onChange={e=>b.setValue(e.target.value)}/></label><output>{p.operation}: {!operandsValid?'Enter finite operands between -1e12 and 1e12':Number.isNaN(value)?'Undefined for these operands':!Number.isFinite(value)?'Outside the finite numeric range':String(value)}</output><p className="genui-muted">Computed locally using JavaScript finite-precision arithmetic.</p></fieldset>;
}
