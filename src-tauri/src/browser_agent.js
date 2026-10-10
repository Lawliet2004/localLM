// In-page driver for the agent browser tab, modeled on browser-use's DOM
// state: every visible interactive element gets a numeric index the model
// acts on. Indexed elements live in window.__llmEls (page realm, no DOM
// mutation) and are invalidated by navigation. Evaluated fresh on every call.
var __llm = (() => {
  const INTERACTIVE = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[contenteditable=""],[contenteditable=true],'
    + '[role=button],[role=link],[role=checkbox],[role=radio],[role=tab],[role=menuitem],[role=option],[role=switch],[role=combobox],[role=textbox],[role=searchbox],[onclick]';
  const MAX_ELEMENTS = 150;
  const clip = (text, n) => { const t = String(text || '').replace(/\s+/g, ' ').trim(); return t.length > n ? t.slice(0, n - 1) + '…' : t; };

  function visible(el) {
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return false;
    // Near-viewport only, like browser-use's viewport expansion: the model
    // scrolls to reach the rest instead of reading a thousand-link dump.
    if (r.bottom < -innerHeight * 0.5 || r.top > innerHeight * 1.5) return false;
    return el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
  }

  function describe(el, i) {
    const tag = el.tagName.toLowerCase();
    const attrs = [];
    for (const name of ['type', 'role', 'name', 'placeholder', 'aria-label', 'title', 'alt']) {
      const value = el.getAttribute(name);
      if (value && !(name === 'type' && tag === 'button')) attrs.push(`${name}="${clip(value, 40)}"`);
    }
    if (tag === 'a') attrs.push(`href="${clip(el.getAttribute('href'), 60)}"`);
    if ((tag === 'input' || tag === 'textarea') && el.type !== 'password' && el.value) attrs.push(`value="${clip(el.value, 40)}"`);
    if (tag === 'select') attrs.push(`value="${clip(el.selectedOptions[0]?.text, 40)}" options="${clip([...el.options].map(o => o.text).join('|'), 80)}"`);
    if (el.checked) attrs.push('checked');
    if (el.disabled) attrs.push('disabled');
    const text = tag === 'select' ? '' : clip(el.innerText || el.textContent, 80);
    return `[${i}]<${tag}${attrs.length ? ' ' + attrs.join(' ') : ''}>${text}</${tag}>`;
  }

  function state() {
    const els = [];
    const lines = [];
    for (const el of document.querySelectorAll(INTERACTIVE)) {
      if (els.length >= MAX_ELEMENTS) break;
      if (!visible(el) || els.some(parent => parent.contains(el))) continue;
      els.push(el);
      lines.push(describe(el, els.length));
    }
    window.__llmEls = els;
    const doc = document.scrollingElement || document.documentElement;
    return {
      url: location.href,
      title: document.title,
      ready: document.readyState === 'complete',
      scroll: `${Math.round(doc.scrollTop)}px above, ${Math.max(0, Math.round(doc.scrollHeight - doc.scrollTop - innerHeight))}px below`,
      elements: lines.join('\n') || '(no interactive elements in view)',
    };
  }

  function element(index) {
    const el = (window.__llmEls || [])[index - 1];
    if (!el || !el.isConnected) throw new Error(`Element [${index}] is gone (the page changed). Use the indices from the latest state.`);
    el.scrollIntoView({ block: 'center', inline: 'center' });
    return el;
  }

  function wrap(action) {
    try { const result = action(); return { ok: true, ...(result || {}) }; } catch (error) { return { ok: false, error: String(error && error.message || error) }; }
  }

  return {
    state: () => wrap(state),
    ready: () => ({ ok: true, ready: document.readyState === 'complete', url: location.href }),
    click: index => wrap(() => { const el = element(index); el.focus?.(); el.click(); }),
    type: (index, text, submit) => wrap(() => {
      const el = element(index);
      el.focus?.();
      if (el.tagName === 'SELECT') {
        const option = [...el.options].find(o => o.text.trim() === text || o.value === text)
          || [...el.options].find(o => o.text.toLowerCase().includes(text.toLowerCase()));
        if (!option) throw new Error(`No option matching "${text}" in [${index}].`);
        el.value = option.value;
      } else if (el.isContentEditable) {
        el.textContent = text;
      } else if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {
        // Native setter so React/Vue controlled inputs see the change.
        const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
        Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, text);
      } else {
        throw new Error(`Element [${index}] is not a text field or select.`);
      }
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      if (submit) {
        const init = { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true };
        const proceed = el.dispatchEvent(new KeyboardEvent('keydown', init));
        el.dispatchEvent(new KeyboardEvent('keyup', init));
        if (proceed && el.form) el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
      }
    }),
    scroll: direction => wrap(() => { scrollBy({ top: (direction === 'up' ? -0.8 : 0.8) * innerHeight }); }),
    read: offset => wrap(() => {
      const text = (document.body?.innerText ?? document.body?.textContent ?? '').replace(/\n{3,}/g, '\n\n');
      const start = Math.max(0, offset || 0);
      const end = start + 12000;
      return { url: location.href, title: document.title, text: text.slice(start, end), totalChars: text.length, nextOffset: end < text.length ? end : null };
    }),
  };
})();
