import { describe, expect, it } from 'vitest';
import { escapeHtml, highlightCode, HIGHLIGHT_MAX_CHARS, languageFromPath, languageLabel, resolveLanguage } from './highlight';

describe('resolveLanguage', () => {
  it('resolves canonical names', () => {
    expect(resolveLanguage('typescript')).toBe('typescript');
    expect(resolveLanguage('python')).toBe('python');
    expect(resolveLanguage('diff')).toBe('diff');
    expect(resolveLanguage('sql')).toBe('sql');
  });

  it('resolves common aliases', () => {
    expect(resolveLanguage('ts')).toBe('typescript');
    expect(resolveLanguage('tsx')).toBe('typescript');
    expect(resolveLanguage('js')).toBe('javascript');
    expect(resolveLanguage('jsx')).toBe('javascript');
    expect(resolveLanguage('py')).toBe('python');
    expect(resolveLanguage('rs')).toBe('rust');
    expect(resolveLanguage('sh')).toBe('bash');
    expect(resolveLanguage('shell')).toBe('bash');
    expect(resolveLanguage('yml')).toBe('yaml');
    expect(resolveLanguage('md')).toBe('markdown');
    expect(resolveLanguage('html')).toBe('xml');
    expect(resolveLanguage('patch')).toBe('diff');
    expect(resolveLanguage('toml')).toBe('ini');
  });

  it('is case- and whitespace-insensitive', () => {
    expect(resolveLanguage('  JSON  ')).toBe('json');
    expect(resolveLanguage('TypeScript')).toBe('typescript');
  });

  it('returns null for missing or unsupported languages', () => {
    expect(resolveLanguage('')).toBeNull();
    expect(resolveLanguage('   ')).toBeNull();
    expect(resolveLanguage(null)).toBeNull();
    expect(resolveLanguage(undefined)).toBeNull();
    expect(resolveLanguage('brainfuck')).toBeNull();
    expect(resolveLanguage('not-a-language')).toBeNull();
  });
});

describe('languageFromPath', () => {
  it('infers a grammar from the file extension', () => {
    expect(languageFromPath('src/main.rs')).toBe('rust');
    expect(languageFromPath('a/b/component.tsx')).toBe('typescript');
    expect(languageFromPath('C:\\repo\\app.py')).toBe('python');
    expect(languageFromPath('styles/site.css')).toBe('css');
    expect(languageFromPath('config.yaml')).toBe('yaml');
    expect(languageFromPath('notes.md')).toBe('markdown');
    expect(languageFromPath('changes.patch')).toBe('diff');
  });

  it('returns null for names without a useful extension', () => {
    expect(languageFromPath('LICENSE')).toBeNull();
    expect(languageFromPath('.env')).toBeNull();
    expect(languageFromPath('archive.')).toBeNull();
    expect(languageFromPath(null)).toBeNull();
    expect(languageFromPath(undefined)).toBeNull();
  });

  it('highlights well-known extensionless build files', () => {
    expect(languageFromPath('Dockerfile')).toBe('dockerfile');
    expect(languageFromPath('dir/Makefile')).toBe('makefile');
    expect(languageFromPath('Dockerfile.dev')).toBe('dockerfile');
  });
});

describe('highlightCode', () => {
  it.each([
    ['cpp', '#include <iostream>\nint main() { return 0; }'],
    ['c', 'int main(void) { return 0; }'],
    ['cs', 'public class App { public int Count = 1; }'],
    ['java', 'public class App { public int count = 1; }'],
    ['go', 'package main\nfunc main() {}'],
    ['kt', 'fun main() { val count = 1 }'],
    ['swift', 'let count = 1'],
    ['rb', 'def count\n  1\nend'],
    ['php', '<?php echo "Hello";'],
    ['ps1', 'Write-Host "Hello"'],
  ])('renders syntax colors for %s', (language, code) => {
    const result = highlightCode(code, language);
    expect(result.language).not.toBeNull();
    expect(result.html).toContain('hljs-');
  });
  it('highlights a supported language with grammar classes', () => {
    const { html, language } = highlightCode('const x = 1;', 'ts');
    expect(language).toBe('typescript');
    expect(html).toContain('hljs-keyword');
    expect(html).toContain('hljs-number');
  });

  it('highlights bash builtins', () => {
    const { html } = highlightCode('echo hi | grep h', 'sh');
    expect(html).toContain('hljs-built_in');
  });

  it('escapes markup inside highlighted source', () => {
    const { html } = highlightCode('<script>alert(1)</script>', 'html');
    expect(html).not.toContain('<script>');
    expect(html).toContain('&lt;');
    expect(html).toContain('script');
  });

  it('falls back to escaped plain text for unknown languages', () => {
    const { html, language } = highlightCode('<b>bold</b>', 'cobol');
    expect(language).toBeNull();
    expect(html).toBe('&lt;b&gt;bold&lt;/b&gt;');
  });

  it('returns plain escaped text when no language is given', () => {
    const { html, language } = highlightCode('plain & <simple>');
    expect(language).toBeNull();
    expect(html).toBe('plain &amp; &lt;simple&gt;');
  });

  it('skips highlighting beyond the size cap and reports truncation', () => {
    const big = `x = ${'1'.repeat(HIGHLIGHT_MAX_CHARS)}`;
    const result = highlightCode(big, 'python');
    expect(result.language).toBeNull();
    expect(result.truncated).toBe(true);
    expect(result.html).toBe(escapeHtml(big));
  });

  it('memoizes repeated highlights of identical input', () => {
    const first = highlightCode('fn main() {}', 'rust');
    const second = highlightCode('fn main() {}', 'rust');
    expect(second).toBe(first);
  });
});

describe('languageLabel', () => {
  it('labels resolved languages canonically', () => {
    expect(languageLabel('ts')).toBe('typescript');
    expect(languageLabel('py')).toBe('python');
  });

  it('keeps the raw label for unresolved languages and defaults to text', () => {
    expect(languageLabel('cobol')).toBe('cobol');
    expect(languageLabel('text')).toBe('text');
    expect(languageLabel(undefined)).toBe('text');
    expect(languageLabel('')).toBe('text');
  });
});
