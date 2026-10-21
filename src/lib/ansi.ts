/**
 * Incremental ANSI text parser for captured command output.
 *
 * Terminal output arrives as one accumulated string per render, so the parser
 * is deterministic over the whole buffer: SGR state carries across line
 * boundaries, carriage-return rewrites resolve to their final visible text,
 * non-SGR control sequences are removed instead of shown as literal junk, and
 * an escape sequence truncated by a split stream chunk is dropped rather than
 * rendered as noise.
 */

const ANSI_FG: Record<number, string> = {
  30: 'var(--terminal-black, #4b5563)',
  31: 'var(--terminal-red, #ef4444)',
  32: 'var(--terminal-green, #10b981)',
  33: 'var(--terminal-yellow, #f59e0b)',
  34: 'var(--terminal-blue, #3b82f6)',
  35: 'var(--terminal-magenta, #ec4899)',
  36: 'var(--terminal-cyan, #06b6d4)',
  37: 'var(--terminal-white, #f3f4f6)',
  90: 'var(--terminal-bright-black, #6b7280)',
  91: 'var(--terminal-bright-red, #f87171)',
  92: 'var(--terminal-bright-green, #34d399)',
  93: 'var(--terminal-bright-yellow, #fbbf24)',
  94: 'var(--terminal-bright-blue, #60a5fa)',
  95: 'var(--terminal-bright-magenta, #f472b6)',
  96: 'var(--terminal-bright-cyan, #22d3ee)',
  97: 'var(--terminal-bright-white, #ffffff)',
};

export interface AnsiSpan {
  text: string;
  color?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
}

export interface AnsiLine {
  spans: AnsiSpan[];
}

interface Style {
  color?: string;
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
}

const PLAIN: Style = { bold: false, dim: false, italic: false, underline: false };

function applySgr(style: Style, code: number): void {
  if (code === 0) {
    style.color = undefined;
    style.bold = false;
    style.dim = false;
    style.italic = false;
    style.underline = false;
  } else if (code === 1) style.bold = true;
  else if (code === 2) style.dim = true;
  else if (code === 3) style.italic = true;
  else if (code === 4) style.underline = true;
  else if (code === 22) { style.bold = false; style.dim = false; }
  else if (code === 23) style.italic = false;
  else if (code === 24) style.underline = false;
  else if (code === 39) style.color = undefined;
  else if (ANSI_FG[code]) style.color = ANSI_FG[code];
  // All other codes (backgrounds, frames, bright alternates we do not theme)
  // are accepted and ignored so they never leak into rendered text.
}

function pushSpan(spans: AnsiSpan[], text: string, style: Style): void {
  if (!text) return;
  const previous = spans[spans.length - 1];
  if (
    previous &&
    previous.color === style.color &&
    previous.bold === style.bold &&
    previous.dim === style.dim &&
    previous.italic === style.italic &&
    previous.underline === style.underline
  ) {
    previous.text += text;
    return;
  }
  spans.push({
    text,
    color: style.color,
    bold: style.bold || undefined,
    dim: style.dim || undefined,
    italic: style.italic || undefined,
    underline: style.underline || undefined,
  });
}

/**
 * Parse terminal output into display lines. Returns one entry per line.
 * Control bytes other than \t are removed; \r rewrites collapse to the text
 * after the last carriage return in each line; ESC-initiated sequences that
 * are not SGR color/style codes are stripped.
 */
export function parseAnsi(input: string): AnsiLine[] {
  const text = input.replace(/\r\n/g, '\n');
  const lines: AnsiLine[] = [];
  const style: Style = { ...PLAIN };
  let spans: AnsiSpan[] = [];
  let buffer = '';
  let i = 0;

  const flushLine = () => {
    pushSpan(spans, buffer, style);
    if (!spans.length) spans.push({ text: '' });
    lines.push({ spans });
    spans = [];
    buffer = '';
  };

  while (i < text.length) {
    const char = text[i];
    if (char === '\x1b') {
      // CSI: ESC [ parameter/intermediate bytes, one final byte. Private
      // markers (? >) are part of the parameter bytes so e.g. ESC[?25l is
      // recognised as one sequence.
      const csi = /^\x1b\[([0-9;:?>]*)([@-~])/.exec(text.slice(i, i + 32));
      if (csi) {
        if (csi[2] === 'm') {
          const codes = csi[1] ? csi[1].split(';').map(part => Number(part.split(':')[0] || 0)) : [0];
          pushSpan(spans, buffer, style);
          buffer = '';
          for (const code of codes) applySgr(style, code);
        }
        // Non-SGR CSI (cursor movement, erase, scroll) drops silently.
        i += csi[0].length;
        continue;
      }
      // OSC: ESC ] ... terminated by BEL or ST. Strip hyperlink/title noise.
      const osc = /^\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/.exec(text.slice(i));
      if (osc) { i += osc[0].length; continue; }
      // A sequence truncated by a split stream chunk (CSI params or OSC body
      // reaching the end of the buffer with no final byte) drops whole.
      if (/^\x1b[\[(][0-9;:?>]*$/.test(text.slice(i)) || /^\x1b\][^\x07\x1b]*$/.test(text.slice(i))) {
        break;
      }
      // Remaining two-byte escapes (ESC letter / ESC ( charset) drop the
      // introducer and one payload byte; a bare trailing ESC drops alone.
      i += text[i + 1] === '(' || text[i + 1] === ')' || text[i + 1] === '#' ? 3 : text[i + 1] ? 2 : 1;
      continue;
    }
    if (char === '\n') {
      flushLine();
      i += 1;
      continue;
    }
    if (char === '\r') {
      // Progress-bar style rewrite: the next segment replaces the visible
      // prefix of this line rather than appending.
      buffer = '';
      spans = [];
      i += 1;
      continue;
    }
    if (char === '\x08') {
      // Backspace deletes the previously emitted character when one exists.
      if (buffer) buffer = buffer.slice(0, -1);
      else if (spans.length) {
        const last = spans[spans.length - 1];
        last.text = last.text.slice(0, -1);
        if (!last.text) spans.pop();
      }
      i += 1;
      continue;
    }
    if (char === '\x07' || char === '\x0b' || char === '\x0c' || char === '\x0e' || char === '\x0f') {
      i += 1;
      continue;
    }
    buffer += char;
    i += 1;
  }
  // Trailing content with no newline still forms a line.
  flushLine();
  return lines;
}

/** Convenience helper: styled spans for a single line of output. */
export function parseAnsiLine(line: string): AnsiSpan[] {
  return parseAnsi(line)[0]?.spans ?? [{ text: '' }];
}
