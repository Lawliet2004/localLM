import hljs from 'highlight.js/lib/core';
import c from 'highlight.js/lib/languages/c';
import cpp from 'highlight.js/lib/languages/cpp';
import csharp from 'highlight.js/lib/languages/csharp';
import java from 'highlight.js/lib/languages/java';
import go from 'highlight.js/lib/languages/go';
import kotlin from 'highlight.js/lib/languages/kotlin';
import swift from 'highlight.js/lib/languages/swift';
import ruby from 'highlight.js/lib/languages/ruby';
import php from 'highlight.js/lib/languages/php';
import dockerfile from 'highlight.js/lib/languages/dockerfile';
import makefile from 'highlight.js/lib/languages/makefile';
import powershell from 'highlight.js/lib/languages/powershell';
import bash from 'highlight.js/lib/languages/bash';
import css from 'highlight.js/lib/languages/css';
import diff from 'highlight.js/lib/languages/diff';
import ini from 'highlight.js/lib/languages/ini';
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import markdown from 'highlight.js/lib/languages/markdown';
import plaintext from 'highlight.js/lib/languages/plaintext';
import python from 'highlight.js/lib/languages/python';
import rust from 'highlight.js/lib/languages/rust';
import sql from 'highlight.js/lib/languages/sql';
import typescript from 'highlight.js/lib/languages/typescript';
import xml from 'highlight.js/lib/languages/xml';
import yaml from 'highlight.js/lib/languages/yaml';

// One shared grammar set for every code surface. Language metadata comes from
// model-provided fence info strings and tool arguments; unrestricted language
// auto-detection is never used, which keeps streaming highlighting bounded.
hljs.registerLanguage('c', c);
hljs.registerLanguage('cpp', cpp);
hljs.registerLanguage('csharp', csharp);
hljs.registerLanguage('java', java);
hljs.registerLanguage('go', go);
hljs.registerLanguage('kotlin', kotlin);
hljs.registerLanguage('swift', swift);
hljs.registerLanguage('ruby', ruby);
hljs.registerLanguage('php', php);
hljs.registerLanguage('dockerfile', dockerfile);
hljs.registerLanguage('makefile', makefile);
hljs.registerLanguage('powershell', powershell);
hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('javascript', javascript);
hljs.registerLanguage('rust', rust);
hljs.registerLanguage('python', python);
hljs.registerLanguage('bash', bash);
hljs.registerLanguage('json', json);
hljs.registerLanguage('xml', xml);
hljs.registerLanguage('css', css);
hljs.registerLanguage('yaml', yaml);
hljs.registerLanguage('sql', sql);
hljs.registerLanguage('markdown', markdown);
hljs.registerLanguage('diff', diff);
hljs.registerLanguage('ini', ini);
hljs.registerLanguage('plaintext', plaintext);

/** Common aliases models and tools emit in fences and arguments. */
const ALIASES: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript',
  js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript', node: 'javascript',
  rs: 'rust',
  py: 'python', py3: 'python', python3: 'python',
  sh: 'bash', shell: 'bash', zsh: 'bash', console: 'bash', shellsession: 'bash', terminal: 'bash',
  json5: 'json', jsonl: 'json', ndjson: 'json',
  html: 'xml', xhtml: 'xml', vue: 'xml', svelte: 'xml', svg: 'xml', hbs: 'xml',
  yml: 'yaml',
  md: 'markdown', mdx: 'markdown',
  patch: 'diff', udiff: 'diff',
  toml: 'ini', conf: 'ini', cfg: 'ini', properties: 'ini', env: 'ini', dotenv: 'ini',
  text: 'plaintext', txt: 'plaintext', log: 'plaintext', plain: 'plaintext',
  'c++': 'cpp', cc: 'cpp', cxx: 'cpp', h: 'c', hpp: 'cpp',
  cs: 'csharp', 'c#': 'csharp', kt: 'kotlin', kts: 'kotlin', rb: 'ruby',
  golang: 'go', pwsh: 'powershell', ps1: 'powershell', docker: 'dockerfile', make: 'makefile',
};

/** Extensions used to infer a grammar for file reads and artifacts. */
const EXTENSIONS: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript',
  js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
  rs: 'rust', py: 'python', pyw: 'python',
  c: 'c', h: 'c', cpp: 'cpp', cc: 'cpp', cxx: 'cpp', hpp: 'cpp',
  cs: 'csharp', java: 'java', go: 'go', kt: 'kotlin', kts: 'kotlin',
  swift: 'swift', rb: 'ruby', php: 'php', ps1: 'powershell',
  sh: 'bash', bash: 'bash', zsh: 'bash',
  json: 'json', jsonl: 'json',
  html: 'xml', htm: 'xml', xhtml: 'xml', xml: 'xml', svg: 'xml',
  css: 'css', scss: 'css', less: 'css',
  yaml: 'yaml', yml: 'yaml', sql: 'sql',
  md: 'markdown', markdown: 'markdown',
  diff: 'diff', patch: 'diff',
  toml: 'ini', ini: 'ini', cfg: 'ini',
  txt: 'plaintext', log: 'plaintext', csv: 'plaintext',
};

/** Above this size the readable plain-text fallback is cheaper than parsing. */
export const HIGHLIGHT_MAX_CHARS = 100_000;
const CACHE_LIMIT = 160;

export function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Resolve a fence info string or tool language argument to a registered
 * grammar id, or null when the surface should render plain text.
 */
export function resolveLanguage(language?: string | null): string | null {
  if (!language) return null;
  const normalized = language.trim().toLowerCase();
  if (!normalized) return null;
  const canonical = ALIASES[normalized] ?? normalized;
  return hljs.getLanguage(canonical) ? canonical : null;
}

/** Guess a grammar from a file name. Returns null when nothing matches. */
export function languageFromPath(path?: string | null): string | null {
  if (!path) return null;
  const name = path.split(/[\\/]/).pop() ?? '';
  const lower = name.toLowerCase();
  if (lower === 'dockerfile' || lower.startsWith('dockerfile.')) return 'dockerfile';
  if (lower === 'makefile' || lower === 'gnumakefile') return 'makefile';
  if (lower === 'cmakelists.txt') return 'plaintext';
  const dot = lower.lastIndexOf('.');
  if (dot <= 0 || dot === lower.length - 1) return null;
  return EXTENSIONS[lower.slice(dot + 1)] ?? null;
}

export interface HighlightResult {
  /** Ready-to-render markup. Grammar output and the fallback are both escaped. */
  html: string;
  /** The grammar actually used, or null for plain text. */
  language: string | null;
  /** True when the size cap skipped highlighting for a large block. */
  truncated?: boolean;
}

const cache = new Map<string, HighlightResult>();

/**
 * Highlight code for display. Highlighted output is HTML produced by
 * highlight.js, which escapes the source; plain fallbacks are escaped here.
 * Results are memoized so repeated renders of unchanged blocks (for example
 * completed blocks while a later block still streams) stay cheap.
 */
export function highlightCode(code: string, language?: string | null): HighlightResult {
  const resolved = resolveLanguage(language);
  if (!resolved) return { html: escapeHtml(code), language: null };
  if (code.length > HIGHLIGHT_MAX_CHARS) return { html: escapeHtml(code), language: null, truncated: true };
  const key = `${resolved}${code}`;
  const hit = cache.get(key);
  if (hit) {
    // Refresh recency for the LRU bound.
    cache.delete(key);
    cache.set(key, hit);
    return hit;
  }
  let result: HighlightResult;
  try {
    result = { html: hljs.highlight(code, { language: resolved, ignoreIllegals: true }).value, language: resolved };
  } catch {
    result = { html: escapeHtml(code), language: null };
  }
  if (cache.size >= CACHE_LIMIT) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  cache.set(key, result);
  return result;
}

/** Short label shown in the code frame toolbar. */
export function languageLabel(language?: string | null): string {
  const resolved = resolveLanguage(language);
  if (!resolved) return language?.trim() || 'text';
  if (resolved === 'plaintext') return 'text';
  return resolved;
}
