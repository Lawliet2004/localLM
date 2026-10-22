import { describe, expect, it } from 'vitest';
import { parseAnsi, parseAnsiLine } from './ansi';

const text = (lines: { spans: { text: string }[] }[]) => lines.map(l => l.spans.map(s => s.text).join(''));

describe('parseAnsi', () => {
  it('returns one line per newline boundary, including a trailing line', () => {
    const lines = parseAnsi('one\ntwo\n');
    expect(text(lines)).toEqual(['one', 'two', '']);
  });

  it('keeps plain text unstyled', () => {
    const [line] = parseAnsi('hello world');
    expect(line.spans).toEqual([{ text: 'hello world' }]);
  });

  it('applies SGR foreground colors and resets them', () => {
    const [line] = parseAnsi('\x1b[31mred\x1b[0m plain');
    expect(line.spans).toEqual([
      { text: 'red', color: 'var(--terminal-red, #ef4444)' },
      { text: ' plain' },
    ]);
  });

  it('applies combined codes from one sequence', () => {
    const [line] = parseAnsi('\x1b[1;32mbold green\x1b[0m');
    expect(line.spans[0]).toMatchObject({ text: 'bold green', bold: true, color: 'var(--terminal-green, #10b981)' });
  });

  it('supports bright colors and attribute codes', () => {
    const [line] = parseAnsi('\x1b[94mbright\x1b[3m italic\x1b[4m under\x1b[0m');
    expect(line.spans[0]).toMatchObject({ color: 'var(--terminal-bright-blue, #60a5fa)' });
    expect(line.spans[1]).toMatchObject({ italic: true });
    expect(line.spans[2]).toMatchObject({ underline: true, italic: true });
  });

  it('carries SGR state across line boundaries', () => {
    const lines = parseAnsi('\x1b[33mfirst\nsecond\x1b[0m\nthird');
    expect(lines[0].spans[0]).toMatchObject({ text: 'first', color: 'var(--terminal-yellow, #f59e0b)' });
    expect(lines[1].spans[0]).toMatchObject({ text: 'second', color: 'var(--terminal-yellow, #f59e0b)' });
    expect(lines[2].spans[0]).toMatchObject({ text: 'third', color: undefined });
  });

  it('honours selective resets (39 default color, 22/23/24 attributes)', () => {
    const [line] = parseAnsi('\x1b[1;31mAB\x1b[39mC\x1b[22mD');
    expect(line.spans.map(s => s.text).join('')).toBe('ABCD');
    expect(line.spans[0]).toMatchObject({ text: 'AB', bold: true });
    expect(line.spans[1]).toMatchObject({ text: 'C', color: undefined, bold: true });
    expect(line.spans[2]).toMatchObject({ text: 'D', color: undefined });
    expect(line.spans[2].bold).toBeFalsy();
  });

  it('collapses carriage-return rewrites to the final segment', () => {
    const lines = parseAnsi('50% done\r100% done');
    expect(text(lines)).toEqual(['100% done']);
  });

  it('normalizes CRLF so carriage returns do not double lines', () => {
    const lines = parseAnsi('a\r\nb');
    expect(text(lines)).toEqual(['a', 'b']);
  });

  it('applies backspace to the previously emitted character', () => {
    const lines = parseAnsi('abc\x08\x08X');
    expect(text(lines)).toEqual(['aX']);
  });

  it('strips cursor, erase and private CSI sequences instead of showing junk', () => {
    const out = text(parseAnsi('\x1b[?25lhidden cursor\x1b[2J\x1b[1;5Hafter'));
    expect(out).toEqual(['hidden cursorafter']);
    expect(out[0]).not.toContain('[?25l');
    expect(out[0]).not.toContain('[2J');
  });

  it('strips OSC sequences such as hyperlinks but keeps the labelled text', () => {
    const out = text(parseAnsi('\x1b]8;;https://example.com\x07link\x1b]8;;\x07 done'));
    expect(out).toEqual(['link done']);
  });

  it('drops a truncated escape sequence left by a split stream chunk', () => {
    const out = text(parseAnsi('kept\x1b[3'));
    expect(out).toEqual(['kept']);
    const out2 = text(parseAnsi('done\x1b'));
    expect(out2).toEqual(['done']);
  });

  it('strips charset and two-byte escapes', () => {
    const out = text(parseAnsi('\x1b(Bplain\x1b7text'));
    expect(out).toEqual(['plaintext']);
  });

  it('preserves tab characters but removes other control bytes', () => {
    const out = text(parseAnsi('a\tb\x0bc\x0cd'));
    expect(out).toEqual(['a\tbcd']);
  });

  it('keeps text styled when the reset arrives on the next line', () => {
    // Simulates a color set mid-stream whose reset lands in a later chunk.
    const lines = parseAnsi('start \x1b[31mcolored\nmore colored\x1b[0m tail');
    expect(lines[0].spans[1]).toMatchObject({ text: 'colored', color: 'var(--terminal-red, #ef4444)' });
    expect(lines[1].spans[0]).toMatchObject({ text: 'more colored', color: 'var(--terminal-red, #ef4444)' });
    expect(lines[1].spans[1]).toMatchObject({ text: ' tail', color: undefined });
  });
});

describe('parseAnsiLine', () => {
  it('returns spans for a single line and a fallback for empty input', () => {
    expect(parseAnsiLine('\x1b[32mok')[0]).toMatchObject({ text: 'ok', color: 'var(--terminal-green, #10b981)' });
    expect(parseAnsiLine('')).toEqual([{ text: '' }]);
  });
});

it('incremental parsing matches replay at every split, including CRLF and OSC terminators', async () => {
  const { AnsiStreamParser, AccumulatedAnsiParser } = await import('./ansi');
  const text = 'start\r\n\x1b[31mred\nnext\x1b[0m\n10%\r90%\n\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\tend\b!';
  for (let split = 0; split <= text.length; split++) {
    const parser = new AnsiStreamParser(); parser.append(text.slice(0, split));
    expect(parser.append(text.slice(split)), `split at ${split}`).toEqual(parseAnsi(text));
    expect(parser.processedChars).toBe(text.length);
  }
  const accumulated = new AccumulatedAnsiParser();
  const before = accumulated.update('done\npartial');
  expect(accumulated.update('done\npartial output')[0]).toBe(before[0]);
  expect(accumulated.update('replacement')).toEqual(parseAnsi('replacement'));
  expect(accumulated.update('')).toEqual([]);
});
