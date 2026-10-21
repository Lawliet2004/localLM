import { describe, expect, it } from 'vitest';
import { GENUI_LIMITS } from './catalog';
import { validateProgram } from './validation';

const program = (body: string) => `root = Response([${body}])`;

describe('bounded OpenUI programs', () => {
  it('accepts supplied sales data exactly', () => {
    expect(validateProgram(program('Table(["Month", "Sales"], [["Jan", 10], ["Feb", 15], ["Mar", 25]])'))).toEqual({ valid: true });
  });
  it('enforces Zod bounds beyond parser type checking', () => {
    for (const body of [
      'Progress("Usage", 101)',
      'Progress("Usage", -1)',
      `Text(${JSON.stringify('x'.repeat(GENUI_LIMITS.textChars + 1))})`,
      'Table(["A", "B"], [[1]])',
      `List(${JSON.stringify(Array(33).fill('item'))})`,
      `Table(["A"], ${JSON.stringify(Array(101).fill([1]))})`,
      'Grid([Text("test")], 5)',
    ]) expect(validateProgram(program(body)), body.slice(0, 90)).toMatchObject({ valid: false });
  });
  it.each([
    'root = Response([Unknown("x")])',
    'root = Response([missing])',
    'root = Response([a])\na = b\nb = a',
    'root = Response([Text("unterminated',
    'root = Text("wrong root")',
    'ordinary Markdown',
    'root = Response([Text(3)])',
    'root = Response([Query("run_shell", {})])',
    'root = Response([])\nx = Mutation("shell", {})',
    '$secret = 1\nroot = Response([])',
    'root = Response([@Repeat("x", 999999999)])',
    'root = Response([Button("Run", {type:"execute",command:"whoami"})])',
    'root = Response([Button("Open", {type:"open_url",url:"javascript:alert(1)"})])',
  ])('rejects invalid or privileged source: %s', source => {
    expect(validateProgram(source).valid).toBe(false);
  });
  it('permits incomplete streams, but rejects them when stopped', () => {
    const source = 'root = Response([t])\nt = Text("Partial';
    expect(validateProgram(source, true)).toEqual({ valid: true });
    expect(validateProgram(source, false).valid).toBe(false);
  });
  it('keeps executable-looking code samples literal', () => {
    const literal = 'Query("shell", {}) @Set($x, 1) <tool_call>{"name":"shell","arguments":{}}</tool_call>';
    expect(validateProgram(program(`CodeBlock(${JSON.stringify(literal)}, "text")`))).toEqual({ valid: true });
  });
  it('bounds source and nesting before parsing', () => {
    expect(validateProgram('x'.repeat(GENUI_LIMITS.sourceChars + 1)).valid).toBe(false);
    expect(validateProgram('root = ' + '['.repeat(1000)).valid).toBe(false);
    expect(validateProgram(')'.repeat(1000) + 'root = ' + '['.repeat(1000))).toMatchObject({ valid: false, diagnostic: expect.stringContaining('unmatched') });
  });
  it('rejects amplified references before materializing an exponential tree', () => {
    let source = 'root = Response(a)\na = [b,b,b,b,b,b,b,b]\n';
    source += 'b = [c,c,c,c,c,c,c,c]\nc = [d,d,d,d,d,d,d,d]\nd = Text("leaf")';
    expect(validateProgram(source)).toMatchObject({ valid: false, diagnostic: expect.stringContaining('resource limit') });
    expect(validateProgram(source.replace('root =', 'ui ='))).toMatchObject({ valid: false, diagnostic: expect.stringContaining('declare root') });
    expect(validateProgram(source + '\na == 1')).toMatchObject({ valid: false, diagnostic: expect.stringContaining('resource limit') });
    expect(validateProgram(source + '\na =')).toMatchObject({ valid: false, diagnostic: expect.stringContaining('Duplicate') });
  });
});
