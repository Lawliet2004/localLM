import { describe, expect, it } from 'vitest';
import driver from '../../src-tauri/src/browser_agent.js?raw';

// Exercises the in-page driver the `browser` harness tool evaluates in the
// agent webview. jsdom has no layout, so rects and checkVisibility are stubbed.
HTMLElement.prototype.getBoundingClientRect = () => ({ width: 10, height: 10, top: 0, bottom: 10 }) as DOMRect;
HTMLElement.prototype.scrollIntoView = () => {};
HTMLElement.prototype.checkVisibility = function checkVisibility(this: HTMLElement) {
  return !this.closest('[style*="display:none"]');
};
// Indirect eval runs in the global scope, like WebView2's ExecuteScript.
// oxlint-disable-next-line no-eval -- evaluating the driver is what's under test
const run = (call: string) => (0, eval)(`${driver}\n;${call}`) as Record<string, unknown>;

describe('browser agent driver', () => {
  it('indexes visible interactive elements and acts on them by index', () => {
    document.body.innerHTML = `<form><input name=q placeholder="Search"><select><option value=a>Apple</option><option value=b>Banana</option></select><button>Go</button></form>
      <a href="/x">Link<button>inner</button></a><div style="display:none"><button>hidden</button></div><input type=password value=secret>`;
    let submitted = 0;
    document.querySelector('form')!.addEventListener('submit', event => { event.preventDefault(); submitted++; });

    expect(run('__llm.state()').elements).toBe([
      '[1]<input name="q" placeholder="Search"></input>',
      '[2]<select value="Apple" options="Apple|Banana"></select>',
      '[3]<button>Go</button>',
      '[4]<a href="/x">Linkinner</a>',
      '[5]<input type="password"></input>',
    ].join('\n'));

    expect(run('__llm.type(1, "hello", true)').ok).toBe(true);
    expect(document.querySelector('input')!.value).toBe('hello');
    expect(submitted).toBe(1);
    run('__llm.type(2, "banana", false)');
    expect(document.querySelector('select')!.value).toBe('b');
    expect(run('__llm.type(3, "x", false)').error).toMatch(/not a text field/);
    expect(run('__llm.click(99)').error).toMatch(/gone/);
    expect(run('__llm.read(0)').text).toContain('Link');
  });
});
