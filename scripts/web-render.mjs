/**
 * Last-resort headless render for the research fetch cascade.
 *
 * Pages behind bot-wall interstitials or heavy client-side rendering return a
 * shell that static extraction cannot read. Instead of bundling Playwright (an
 * extra browser download and container-weight dependency), we reuse the Edge
 * installation that WebView2 already requires on Windows: `msedge --headless`
 * renders the page and dumps the post-script DOM.
 *
 * The renderer re-checks the SSRF guard itself: the browser resolves DNS on
 * its own, so the pinned-address check from web-transport must run before
 * spawn. Rendered output is untrusted page text; it flows through the same
 * extraction and sanitization path as a normal fetch.
 */
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { addressesFor } from './web-transport.mjs';

const EDGE_CANDIDATES = [
  process.env.LOCALLM_EDGE_PATH,
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  process.env.LOCALAPPDATA ? join(process.env.LOCALAPPDATA, 'Microsoft\\Edge\\Application\\msedge.exe') : undefined,
].filter(Boolean);

export function findEdgeBinary(candidates = EDGE_CANDIDATES, exists = existsSync) {
  return candidates.find((path) => exists(path)) || null;
}

/**
 * Render `url` in headless Edge and return the serialized DOM.
 * @returns {Promise<{html: string, finalUrl: string} | null>} null when Edge is
 * unavailable, the render times out, or the page yields no usable markup.
 */
export async function renderPage(url, { timeoutMs = 20000, maxBytes = 5 * 1024 * 1024, virtualTimeBudgetMs = 8000, edgePath } = {}) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  // The browser bypasses the pinned-DNS fetch path, so the public-address
  // check must be repeated against the render target.
  try {
    await addressesFor(parsed);
  } catch {
    return null;
  }
  const binary = edgePath || findEdgeBinary();
  if (!binary) return null;

  const profile = mkdtempSync(join(tmpdir(), 'locallm-render-'));
  try {
    const html = await dumpDom(binary, url, profile, { timeoutMs, maxBytes, virtualTimeBudgetMs });
    return html && html.length >= 150 ? { html, finalUrl: url } : null;
  } catch {
    return null;
  } finally {
    rmSync(profile, { recursive: true, force: true });
  }
}

function dumpDom(binary, url, profile, { timeoutMs, maxBytes, virtualTimeBudgetMs }) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, [
      '--headless=new',
      '--disable-gpu',
      '--disable-extensions',
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-background-networking',
      '--disable-sync',
      '--mute-audio',
      `--user-data-dir=${profile}`,
      `--virtual-time-budget=${virtualTimeBudgetMs}`,
      '--dump-dom',
      url,
    ], { stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true });

    let size = 0;
    let overflow = false;
    const parts = [];
    child.stdout.on('data', (chunk) => {
      size += chunk.length;
      if (size > maxBytes) {
        overflow = true;
        child.kill();
      } else {
        parts.push(chunk);
      }
    });
    const timer = setTimeout(() => child.kill(), timeoutMs);
    child.on('error', reject);
    child.on('close', () => {
      clearTimeout(timer);
      if (overflow) {
        reject(new Error('Rendered DOM exceeds byte limit'));
      } else {
        resolve(Buffer.concat(parts).toString('utf8'));
      }
    });
  });
}
