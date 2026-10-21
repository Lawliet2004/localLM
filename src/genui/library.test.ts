import { describe, expect, it } from 'vitest';
import { createParser } from '@openuidev/react-lang';
import { locallmLibrary } from './library';
import { GENUI_ROOT, genuiComponents } from './catalog';

const parser = createParser(locallmLibrary.toJSONSchema(), GENUI_ROOT);

function parse(program: string) {
  const result = parser.parse(program);
  const fatal = result.meta.errors.filter(error => error.severity !== 'warning');
  return { result, fatal };
}

describe('locallmLibrary', () => {
  it('registers every catalog component under the Response root', () => {
    expect(locallmLibrary.root).toBe(GENUI_ROOT);
    for (const component of genuiComponents) {
      expect(locallmLibrary.components[component.name], component.name).toBeDefined();
    }
    const names = genuiComponents.map(component => component.name);
    expect(new Set(names).size).toBe(names.length);
  });

  it('generates model instructions covering every component', () => {
    const prompt = locallmLibrary.prompt();
    expect(prompt).toContain('Component Signatures');
    for (const component of genuiComponents) {
      expect(prompt).toContain(component.name);
    }
  });
});

describe('OpenUI Lang programs parse against the library', () => {
  it('parses a markdown + stat program', () => {
    const { result, fatal } = parse(
      'root = Response([summary, stats])\n' +
      'summary = Markdown("**Done.**") \n' +
      'stats = Columns([Stat("Downloaded", "2.07 GB"), Stat("Errors", "0")])',
    );
    expect(fatal).toEqual([]);
    expect(result.root).not.toBeNull();
  });

  it('parses tables, key/value lists and callouts', () => {
    const { result, fatal } = parse(
      'root = Response([section])\n' +
      'section = Section("Report", [t, kv, note])\n' +
      't = Table(["A", "B"], [["1", "2"], ["3", "4"]])\n' +
      'kv = KeyValue([{ label: "Model", value: "Alpha" }])\n' +
      'note = Callout("All checks passed", "success")',
    );
    expect(fatal).toEqual([]);
    expect(result.root).not.toBeNull();
  });

  it('parses buttons, progress and code blocks', () => {
    const { result, fatal } = parse(
      'root = Response([p, c, b, l])\n' +
      'p = Progress("Indexing", 62, "1,240 of 2,000 files")\n' +
      'c = CodeBlock("print(1)", "python")\n' +
      'b = Button("Tell me more", { type: "continue_conversation" })\n' +
      'l = Button("Docs", { type: "open_url", url: "https://example.com" }, "primary")',
    );
    expect(fatal).toEqual([]);
    expect(result.root).not.toBeNull();
  });

  it('tolerates a truncated streaming program', () => {
    const streaming = createParser(locallmLibrary.toJSONSchema(), GENUI_ROOT);
    const result = streaming.parse('root = Response([summary\nsummary = Markdown("Partial');
    expect(result.meta.incomplete).toBe(true);
    expect(result.root).not.toBeNull();
  });

  it('reports no root for prose input', () => {
    const { result } = parse('just some text, no program');
    expect(result.root).toBeNull();
  });
});
