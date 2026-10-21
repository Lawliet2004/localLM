/** Parse public structured documents without treating their contents as HTML. */
export function extractStructuredContent(body: string, mime: string) {
  const source = body.replace(/^\uFEFF/, '');
  if (source.length > 5 * 1024 * 1024) throw new Error('Structured document exceeds 5 MiB.');
  if (mime === 'application/json' || mime.endsWith('+json')) {
    const value: unknown = JSON.parse(source);
    const text = JSON.stringify(value, null, 2);
    if (text.length > 10 * 1024 * 1024) throw new Error('Expanded JSON exceeds 10 MiB.');
    return { text, extractionMethod: 'structured_json', rows: Array.isArray(value) ? value.length : undefined };
  }
  const delimiter = mime === 'text/tab-separated-values' ? '\t' : ',';
  const rows: string[][] = [];
  let row: string[] = [], cell = '', quoted = false, afterQuote = false;
  function field() {
    row.push(cell); cell = ''; afterQuote = false;
    if (row.length > 256) throw new Error('Delimited document exceeds 256 columns.');
  }
  function line() {
    field(); rows.push(row); row = [];
    if (rows.length > 10000) throw new Error('Delimited document exceeds 10000 rows. Request a smaller file.');
  }
  for (let index = 0; index < source.length; index++) {
    const char = source[index];
    if (quoted) {
      if (char === '"' && source[index + 1] === '"') { cell += '"'; index++; }
      else if (char === '"') { quoted = false; afterQuote = true; }
      else cell += char;
    } else if (char === delimiter) field();
    else if (char === '\n' || char === '\r') { if (char === '\r' && source[index + 1] === '\n') index++; line(); }
    else if (char === '"' && !cell && !afterQuote) quoted = true;
    else if (afterQuote || char === '"') throw new Error('Malformed delimited document: unexpected text around a quoted field.');
    else cell += char;
  }
  if (quoted) throw new Error('Malformed delimited document: unterminated quoted field.');
  if (cell || row.length || afterQuote) line();
  return { text: rows.map((cells, index) => `Row ${index + 1}: ${JSON.stringify(cells)}`).join('\n'), extractionMethod: delimiter === '\t' ? 'structured_tsv' : 'structured_csv', rows: rows.length };
}
