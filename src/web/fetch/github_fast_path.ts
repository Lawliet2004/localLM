/**
 * GitHub fast path: github.com pages are heavy JS shells that the raw,
 * no-render fetcher extracts poorly, while the underlying content is freely
 * available as plain text. Blob URLs map deterministically onto
 * raw.githubusercontent.com; repository roots map onto the README at the
 * default branch. Failures fall back to the ordinary HTML fetch, so the fast
 * path can only add content, never remove it.
 */

const GITHUB_HOST = 'github.com';
const RAW_HOST = 'https://raw.githubusercontent.com';

export interface GitHubRawCandidate {
  rawUrl: string;
  /** Human content type, used for extraction hints. */
  kind: 'readme' | 'blob';
}

function safeSegment(value: string): boolean {
  return value.length > 0 && !value.includes('\\') && !/^\.+$/.test(value);
}

/** Map a github.com URL onto its raw content URL(s); null when not applicable. */
export function toGitHubRawCandidates(url: string): GitHubRawCandidate[] | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.hostname !== GITHUB_HOST || parsed.protocol !== 'https:') return null;
  const segments = parsed.pathname.split('/').filter((s) => s.length > 0);
  // Minimum: /{owner}/{repo}. Skip special routes (settings, topics, orgs...).
  if (segments.length < 2 || !segments.every(safeSegment)) return null;
  const reserved = new Set(['settings', 'topics', 'orgs', 'organizations', 'marketplace', 'pulls', 'issues', 'notifications', 'explore', 'trending', 'features', 'security', 'pricing', 'sponsors', 'collections']);
  if (reserved.has(segments[0].toLowerCase())) return null;
  const [owner, repo, third, fourth, ...rest] = segments;

  // Repository root: try the README at the common default branches.
  if (segments.length === 2) {
    return [
      { rawUrl: `${RAW_HOST}/${owner}/${repo}/HEAD/README.md`, kind: 'readme' },
      { rawUrl: `${RAW_HOST}/${owner}/${repo}/main/README.md`, kind: 'readme' },
      { rawUrl: `${RAW_HOST}/${owner}/${repo}/master/README.md`, kind: 'readme' },
    ];
  }

  // File view: /{owner}/{repo}/blob/{ref}/{path...}
  if (third === 'blob' && fourth && rest.length >= 0) {
    const path = [fourth, ...rest].map(encodeURIComponent).join('/');
    return [{ rawUrl: `${RAW_HOST}/${owner}/${repo}/${path}`, kind: 'blob' }];
  }

  // Raw links already point at raw content; everything else (trees, releases,
  // issues...) stays on the ordinary fetch path.
  return null;
}

/** True when a fast-path candidate response actually carries usable content. */
export function isUsableRawResponse(status: number | undefined, mime: string | undefined, body: string | undefined, kind: GitHubRawCandidate['kind']): boolean {
  if (status !== 200 || !body) return false;
  if (kind === 'blob') return body.trim().length > 0;
  // README candidates 404 with "400: Invalid request" style bodies on raw when
  // the default branch or file name differs; require a plausible README size.
  const texty = !mime || /^(text\/(plain|markdown)|application\/octet-stream)/.test(mime);
  return texty && body.trim().length > 60;
}
