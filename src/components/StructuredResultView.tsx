import type { ReactNode } from 'react';

// Friendly rendering for structured tool results. The generic Result tab used
// to be a raw envelope dump (`schemaVersion`, `metadata.sideEffects`, stray
// braces); this view renders the payload — key/value rows for scalars, tables
// for entry lists — and hides envelope bookkeeping. Nothing is lost: callers
// keep the raw JSON one disclosure away.

const ENVELOPE_KEYS = new Set(['schemaVersion', 'status', 'isError', 'metadata', 'artifacts']);
const MAX_TABLE_COLUMNS = 6;
const MAX_TABLE_ROWS = 50;
const MAX_CELL_CHARS = 120;
const MAX_STRING_INLINE = 400;
const MAX_DEPTH = 3;

function isEnvelopeKey(key: string, value: unknown): boolean {
  // `error` stays visible when it carries a message; a null error is noise.
  if (key === 'error') return value === null || value === undefined || value === '';
  return ENVELOPE_KEYS.has(key) || key.startsWith('_');
}

function scalarText(value: unknown): string | null {
  if (value === null) return 'null';
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return null;
}

function cellText(value: unknown): string {
  const scalar = scalarText(value);
  const text = scalar ?? JSON.stringify(value) ?? '';
  return text.length > MAX_CELL_CHARS ? `${text.slice(0, MAX_CELL_CHARS)}…` : text;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function ObjectTable({ items }: { items: Record<string, unknown>[] }) {
  const allColumns = [...new Set(items.flatMap(item => Object.keys(item)))];
  const columns = allColumns.slice(0, MAX_TABLE_COLUMNS);
  const rows = items.slice(0, MAX_TABLE_ROWS);
  const hiddenColumns = allColumns.length - columns.length;
  const hiddenRows = items.length - rows.length;
  return (
    <div className="structured-result-table">
      <table>
        <thead>
          <tr>
            {columns.map(column => <th key={column}>{column}</th>)}
          </tr>
        </thead>
        <tbody>
          {rows.map((item, index) => (
            <tr key={index}>
              {columns.map(column => <td key={column}>{cellText(item[column])}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
      {(hiddenRows > 0 || hiddenColumns > 0) && (
        <p className="structured-result-more">
          {hiddenRows > 0 ? `${hiddenRows} more rows. ` : ''}
          {hiddenColumns > 0 ? `${hiddenColumns} more columns in the raw result. ` : ''}
          Expand the raw result below for everything.
        </p>
      )}
    </div>
  );
}

function ValueView({ value }: { value: unknown }) {
  const scalar = scalarText(value);
  if (scalar !== null) {
    if (scalar.length > MAX_STRING_INLINE || scalar.includes('\n')) {
      return <pre className="structured-result-text">{scalar}</pre>;
    }
    return <code className="structured-result-scalar">{scalar}</code>;
  }
  const encoded = JSON.stringify(value) ?? '';
  return <pre className="structured-result-text">{encoded}</pre>;
}

function renderEntry(key: string, value: unknown, depth: number): ReactNode {
  if (Array.isArray(value)) {
    if (value.length > 0 && value.every(isPlainObject)) {
      return (
        <div className="structured-result-section" key={key}>
          <p className="structured-result-heading">{key} <span className="structured-result-count">({value.length})</span></p>
          <ObjectTable items={value} />
        </div>
      );
    }
    const joined = value.map(item => cellText(item)).join(', ');
    const clipped = joined.length > MAX_STRING_INLINE ? `${joined.slice(0, MAX_STRING_INLINE)}…` : joined;
    return (
      <div className="param-row" key={key}>
        <dt className="param-key">{key}</dt>
        <dd className="param-value"><code className="structured-result-scalar">{clipped}</code></dd>
      </div>
    );
  }
  if (isPlainObject(value)) {
    const visible = Object.entries(value).filter(([child, childValue]) => !isEnvelopeKey(child, childValue));
    if (depth >= MAX_DEPTH || visible.length === 0) {
      return (
        <div className="param-row" key={key}>
          <dt className="param-key">{key}</dt>
          <dd className="param-value"><ValueView value={value} /></dd>
        </div>
      );
    }
    return (
      <div className="structured-result-section" key={key}>
        <p className="structured-result-heading">{key}</p>
        {visible.map(([child, childValue]) => renderEntry(child, childValue, depth + 1))}
      </div>
    );
  }
  return (
    <div className="param-row" key={key}>
      <dt className="param-key">{key}</dt>
      <dd className="param-value"><ValueView value={value} /></dd>
    </div>
  );
}

export function StructuredResultView({ result }: { result: Record<string, unknown> }) {
  const visible = Object.entries(result).filter(([key, value]) => !isEnvelopeKey(key, value));
  if (visible.length === 0) {
    return <p className="structured-result-more">Only envelope metadata in this result; expand the raw result below.</p>;
  }
  return (
    <dl className="structured-result param-list">
      {visible.map(([key, value]) => renderEntry(key, value, 0))}
    </dl>
  );
}
