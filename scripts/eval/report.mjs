// Aggregation and rendering for eval runs. Repeats are summarised with their
// spread: a small model's run-to-run variance usually exceeds the deltas being
// measured, so a single number without it is not reported.

export function summarize(values) {
  const measured = values.filter(Number.isFinite);
  if (!measured.length) return { n: 0, mean: null, sd: null, min: null, max: null };
  const mean = measured.reduce((sum, value) => sum + value, 0) / measured.length;
  const sd = measured.length > 1
    ? Math.sqrt(measured.reduce((sum, value) => sum + (value - mean) ** 2, 0) / (measured.length - 1))
    : null;
  return { n: measured.length, mean, sd, min: Math.min(...measured), max: Math.max(...measured) };
}

const METRICS = ['wallMs', 'toolCalls', 'toolCallValidity', 'toolErrors', 'approvals', 'peakInputTokens', 'outputTokens', 'cacheHitRatio'];

function group(runs, keyOf) {
  const groups = new Map();
  for (const run of runs) {
    const key = keyOf(run);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(run);
  }
  return groups;
}

function aggregateGroup(runs) {
  const metrics = {};
  for (const metric of METRICS) metrics[metric] = summarize(runs.map(run => run.metrics[metric]));
  return {
    runs: runs.length,
    passed: runs.filter(run => run.passed).length,
    successRate: summarize(runs.map(run => (run.passed ? 1 : 0))),
    metrics,
  };
}

/** Per (model, preset, task) and per (model, preset) aggregates. */
export function aggregate(runs) {
  const byTask = [];
  for (const [, items] of group(runs, run => `${run.model}\u0000${run.preset}\u0000${run.taskId}`)) {
    byTask.push({ model: items[0].model, preset: items[0].preset, taskId: items[0].taskId, ...aggregateGroup(items) });
  }
  const byConfig = [];
  for (const [, items] of group(runs, run => `${run.model}\u0000${run.preset}`)) {
    byConfig.push({ model: items[0].model, preset: items[0].preset, tasks: new Set(items.map(run => run.taskId)).size, ...aggregateGroup(items) });
  }
  return { byTask, byConfig };
}

function fmt(stat, digits = 2, scale = 1) {
  if (!stat || stat.n === 0) return 'n/a';
  const mean = (stat.mean * scale).toFixed(digits);
  return stat.sd == null ? mean : `${mean} ± ${(stat.sd * scale).toFixed(digits)}`;
}

export function renderMarkdown(report) {
  const { byTask, byConfig } = aggregate(report.runs);
  const lines = [
    `# Eval report`,
    '',
    `- Started: ${report.startedAt}`,
    `- Seed: ${report.seed ?? 'none (sampling not reproducible)'}`,
    `- Repeats per task: ${report.repeats}`,
    `- Values are mean ± sample SD over repeats. Output tokens and cache reuse come from llama-server timings; n/a where the provider reports none.`,
    '',
    '## By configuration',
    '',
    '| Model | Preset | Tasks | Pass | Success rate | Tool-call validity | Tool calls | Wall s | Peak input tokens | Output tokens | Cache hit % |',
    '|---|---|---|---|---|---|---|---|---|---|---|',
    ...byConfig.map(row => `| ${row.model} | ${row.preset} | ${row.tasks} | ${row.passed}/${row.runs} | ${fmt(row.successRate, 0, 100)}% | ${fmt(row.metrics.toolCallValidity)} | ${fmt(row.metrics.toolCalls, 1)} | ${fmt(row.metrics.wallMs, 1, 0.001)} | ${fmt(row.metrics.peakInputTokens, 0)} | ${fmt(row.metrics.outputTokens, 0)} | ${fmt(row.metrics.cacheHitRatio, 0, 100)} |`),
    '',
    '## By task',
    '',
    '| Model | Preset | Task | Pass | Tool-call validity | Tool calls | Tool errors | Approvals | Wall s |',
    '|---|---|---|---|---|---|---|---|---|',
    ...byTask.map(row => `| ${row.model} | ${row.preset} | ${row.taskId} | ${row.passed}/${row.runs} | ${fmt(row.metrics.toolCallValidity)} | ${fmt(row.metrics.toolCalls, 1)} | ${fmt(row.metrics.toolErrors, 1)} | ${fmt(row.metrics.approvals, 1)} | ${fmt(row.metrics.wallMs, 1, 0.001)} |`),
  ];
  if (report.skipped?.length) {
    lines.push('', '## Skipped', '');
    for (const skip of report.skipped) lines.push(`- ${skip.taskId} (${skip.preset}): ${skip.reason}`);
  }
  const failures = report.runs.filter(run => !run.passed);
  if (failures.length) {
    lines.push('', '## Failures', '');
    for (const run of failures) {
      const failed = run.assertions.filter(assertion => !assertion.pass).map(assertion => `${assertion.type}: ${assertion.detail}`);
      lines.push(`- **${run.taskId}** (${run.preset}, repeat ${run.repeat}) — status ${run.metrics.status}${run.metrics.error ? `, error: ${run.metrics.error}` : ''}${failed.length ? `; ${failed.join('; ')}` : ''}`);
    }
  }
  return lines.join('\n') + '\n';
}
