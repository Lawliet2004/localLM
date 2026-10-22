/**
 * Incremental ANSI text parser for captured command output.
 *
 * Both accumulated history and incremental live output use one parser: SGR state carries across line
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

/** One parser per output channel. Completed lines retain their identities;
 * only newly appended bytes and the unfinished line are processed. */
export class AnsiStreamParser {
  private style: Style = { ...PLAIN };
  private completed: AnsiLine[] = [];
  private spans: AnsiSpan[] = [];
  private buffer = '';
  private pending = '';
  processedChars = 0;

  append(chunk: string): AnsiLine[] {
    this.processedChars += chunk.length;
    const text = (this.pending + chunk).replace(/\r\n/g, '\n');
    this.pending = '';
    let i = 0;
    while (i < text.length) {
      const char = text[i];
      if (char === '\x1b') {
        const rest = text.slice(i);
        const csi = /^\x1b\[([0-9;:?>]*)([@-~])/.exec(rest.slice(0, 32));
        if (csi) {
          if (csi[2] === 'm') {
            pushSpan(this.spans, this.buffer, this.style); this.buffer = '';
            for (const code of csi[1] ? csi[1].split(';').map(part => Number(part.split(':')[0] || 0)) : [0]) applySgr(this.style, code);
          }
          i += csi[0].length; continue;
        }
        const osc = /^\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/.exec(rest);
        if (osc) { i += osc[0].length; continue; }
        if (rest === '\x1b' || /^\x1b[\[(][0-9;:?>]*$/.test(rest) || /^\x1b\][^\x07\x1b]*(?:\x1b)?$/.test(rest)) {
          this.pending = rest; break;
        }
        i += text[i + 1] === '(' || text[i + 1] === ')' || text[i + 1] === '#' ? 3 : 2;
        continue;
      }
      if (char === '\n') {
        pushSpan(this.spans, this.buffer, this.style);
        this.completed.push({ spans: this.spans.length ? this.spans : [{ text: '' }] });
        this.spans = []; this.buffer = '';
      } else if (char === '\r') {
        if (i === text.length - 1) { this.pending = char; break; }
        this.buffer = ''; this.spans = [];
      } else if (char === '\x08') {
        if (this.buffer) this.buffer = this.buffer.slice(0, -1);
        else if (this.spans.length) {
          const last = this.spans[this.spans.length - 1];
          last.text = last.text.slice(0, -1); if (!last.text) this.spans.pop();
        }
      } else if (char === '\t' || char.charCodeAt(0) >= 32 && char !== '\x7f') {
        this.buffer += char;
      }
      i++;
    }
    return this.snapshot();
  }

  snapshot(): AnsiLine[] {
    const tail = this.pending === '\r' ? [] : this.spans.map(span => ({ ...span }));
    if (this.pending !== '\r') pushSpan(tail, this.buffer, this.style);
    return [...this.completed, { spans: tail.length ? tail : [{ text: '' }] }];
  }
}

/** Adapter for accumulated React props; replacements/replays reset safely. */
export class AccumulatedAnsiParser {
  private source = '';
  private parser = new AnsiStreamParser();
  private lines: AnsiLine[] = [];
  update(source: string): AnsiLine[] {
    if (!source) { this.source = ''; this.parser = new AnsiStreamParser(); this.lines = []; return this.lines; }
    if (source === this.source) return this.lines;
    if (!source.startsWith(this.source)) { this.parser = new AnsiStreamParser(); this.source = ''; }
    this.lines = this.parser.append(source.slice(this.source.length));
    this.source = source;
    return this.lines;
  }
}

export function parseAnsi(input: string): AnsiLine[] {
  return new AnsiStreamParser().append(input);
}

export function parseAnsiLine(line: string): AnsiSpan[] {
  return parseAnsi(line)[0]?.spans ?? [{ text: '' }];
}
