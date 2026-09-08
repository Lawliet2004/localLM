import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';

// Run deliberately when reviewing upstream updates. Runtime installs only this lockfile.
const revisions = {
  'anthropics/skills': '41bbe19d1a1a7eaab5e7bb9050a417e5c6cffc8f',
  'tavily-ai/skills': '778122e5f9c680f541eeceda5a5b36405eb7980c',
  'supabase/agent-skills': '8331f910845103c08d51f6ca1d86ebb7d1f745e3',
  'microsoft/skills': '02e0b2f852b39ea00c43283f999b83fc12079273',
  'openai/skills': '49f948faa9258a0c61caceaf225e179651397431',
};
const catalog = JSON.parse(readFileSync('src/lib/catalog.json', 'utf8')).skills;
const trees = new Map();
for (const [repo, revision] of Object.entries(revisions)) {
  const response = await fetch(`https://api.github.com/repos/${repo}/git/trees/${revision}?recursive=1`, { headers: { 'User-Agent': 'LocalLM-skill-lock' } });
  if (!response.ok) throw new Error(`${repo}: HTTP ${response.status}`);
  const tree = await response.json();
  if (tree.truncated) throw new Error(`Incomplete tree for ${repo}`);
  trees.set(repo, tree.tree);
}
const locks = [];
for (const skill of catalog) {
  const repo = skill.url.replace('https://github.com/', '');
  const revision = revisions[repo];
  const tree = trees.get(repo);
  const entries = tree.filter(entry => entry.type === 'blob' && entry.path.startsWith(`${skill.path}/`));
  const rootLicense = tree.find(entry => entry.type === 'blob' && /^licen[cs]e(?:\.[^.]+)?$/i.test(entry.path));
  if (rootLicense) entries.push(rootLicense);
  if (!entries.some(entry => entry.path === `${skill.path}/SKILL.md`)) throw new Error(`Missing SKILL.md: ${skill.name}`);
  if (entries.length > 512) throw new Error(`Too many files: ${skill.name}`);
  const files = [];
  let cursor = 0;
  await Promise.all(Array.from({ length: 6 }, async () => {
    while (cursor < entries.length) {
      const entry = entries[cursor++];
      if (!['100644', '100755'].includes(entry.mode)) throw new Error(`Unsafe entry: ${entry.path}`);
      const response = await fetch(`https://raw.githubusercontent.com/${repo}/${revision}/${entry.path}`);
      if (!response.ok) throw new Error(`${entry.path}: HTTP ${response.status}`);
      const bytes = Buffer.from(await response.arrayBuffer());
      if (bytes.length > 4 * 1024 * 1024) throw new Error(`Oversized file ${entry.path}`);
      const gitHash = createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');
      if (gitHash !== entry.sha) throw new Error(`Git blob mismatch ${entry.path}`);
      files.push({ path: entry === rootLicense ? 'UPSTREAM-LICENSE' : entry.path.slice(skill.path.length + 1), sourcePath: entry.path, size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') });
    }
  }));
  files.sort((a, b) => a.path.localeCompare(b.path));
  locks.push({ id: skill.name, description: skill.description, repo, revision, sourcePath: skill.path, files });
  console.log(`${skill.name}: ${files.length} verified files, ${files.reduce((sum, file) => sum + file.size, 0)} bytes`);
}
writeFileSync('catalog/skills.lock.json', JSON.stringify({ version: 1, skills: locks }, null, 2) + '\n');
