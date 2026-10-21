import { useEffect, useState, useRef } from "react";
import { api } from "../lib/api";
import type { PresentationMode } from "../lib/types";
import { savePresentationDefault } from "../hooks/usePresentationMode";
export function PresentationDefaults() {
  const [mode, setMode] = useState<PresentationMode>("auto"),
    [error, setError] = useState("");
  const revision = useRef(0);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    const token = ++revision.current;
    if (api.getPresentationMode)
      void api
        .getPresentationMode(null)
        .then((value) => {
          if (value && revision.current === token) setMode(value.defaultMode);
        })
        .catch(() => {});
    return () => { ++revision.current; };
  }, []);
  return (
    <div className="preference-row">
      <div className="preference-copy">
        <label htmlFor="presentation-default">Response presentation</label>
        <p>
          Auto uses visuals for explicit diagrams, comparisons and interactive
          tasks. Text is fastest. Visual formats the completed answer with the
          selected model.
        </p>
        {error && <p role="alert">{error}</p>}
      </div>
      <select
        id="presentation-default"
        value={mode}
        disabled={saving}
        onChange={async (e) => {
          if (saving) return;
          const next = e.target.value as PresentationMode;
          const token = ++revision.current;
          setSaving(true);
          try {
            await savePresentationDefault(next);
            if (revision.current !== token) return;
            setMode(next);
            setError("");
          } catch {
            if (revision.current === token) setError("Could not save response presentation.");
          } finally {
            if (revision.current === token) setSaving(false);
          }
        }}
      >
        <option value="auto">Auto</option>
        <option value="text">Text</option>
        <option value="visual">Visual</option>
      </select>
    </div>
  );
}
