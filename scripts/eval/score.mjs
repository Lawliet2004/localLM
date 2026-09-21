// Pure scoring over a recorded trajectory. No app access here, so every
// function is unit-testable (score.test.mjs) and re-runnable on saved reports.
import { resolve, sep } from 'node:path';

// Terminal turn errors the chat loop raises when the model emits a tool call
// it cannot execute (chat.rs, tool_calls.rs). Each one ends the turn, so a
// failed turn contributes at most one malformed call.
export const MALFORMED_TOOL_CALL_ERRORS = [
  'text-form tool call',
  'Invalid tool call array',
  'Tool call index is missing',
  'Malformed tool call fragment',
  'more than eight tools in one round',
  'Tool call exceeds its size limit',
  'missing or duplicate identifiers',
  'Tool arguments are empty',
  'Tool arguments are not valid JSON',
];

export function isMalformedToolCallError(error) {
  return typeof error === 'string' && MALFORMED_TOOL_CALL_ERRORS.some(fragment => error.includes(fragment));
}

// Versioned envelopes (tool_envelope.rs) carry `status`; legacy execution
// results carry `isError`. Anything else is not counted as a failure.
const ENVELOPE_FAILURES = ['failure', 'cancelled', 'unknown'];
function resultIsError(result) {
  if (!result || typeof result !== 'object') return false;
  return result.isError === true || ENVELOPE_FAILURES.includes(result.status);
}

/** Metrics from `session_events` for one turn, plus runner-observed data. */
export function scoreTrajectory(events, observed = {}) {
  const calls = events.filter(event => event.eventType === 'tool_call');
  const results = events.filter(event => event.eventType === 'tool_result');
  const turnEnd = [...events].reverse().find(event => event.eventType === 'turn_end');
  const status = turnEnd?.payload?.status ?? (observed.timedOut ? 'timeout' : 'unknown');
  const error = turnEnd?.payload?.error ?? observed.error ?? null;
  const malformed = isMalformedToolCallError(error) ? 1 : 0;
  const attempted = calls.length + malformed;
  const byName = {};
  for (const call of calls) byName[call.payload?.name] = (byName[call.payload?.name] ?? 0) + 1;
  const inputTokens = (observed.contexts ?? []).map(context => context.inputTokens).filter(Number.isFinite);
  // Runtime-measured per-round timings (telemetry.rs). Absent for providers
  // that report none, in which case these stay null rather than estimated.
  const timings = events.filter(event => event.eventType === 'timings').map(event => event.payload ?? {});
  const sum = key => {
    const values = timings.map(timing => timing[key]).filter(Number.isFinite);
    return values.length ? values.reduce((total, value) => total + value, 0) : null;
  };
  const cached = sum('cacheN');
  const evaluated = sum('promptN');
  const restore = [...events].reverse().find(event => event.eventType === 'kv_slot' && event.payload?.action === 'restore');
  return {
    status,
    error,
    rounds: events.filter(event => event.eventType === 'step_start').length,
    toolCalls: calls.length,
    toolCallsByName: byName,
    malformedToolCalls: malformed,
    // null, not 1.0, when the model attempted no calls: nothing was measured.
    toolCallValidity: attempted ? calls.length / attempted : null,
    toolErrors: results.filter(event => resultIsError(event.payload?.result)).length,
    deniedCalls: results.filter(event => event.payload?.decision === 'denied').length,
    approvals: (observed.approvals ?? []).length,
    // Largest measured prompt size in the turn.
    peakInputTokens: inputTokens.length ? Math.max(...inputTokens) : null,
    inputTokensEstimated: (observed.contexts ?? []).some(context => context.estimated === true),
    outputTokens: sum('predictedN'),
    promptTokensEvaluated: evaluated,
    promptTokensCached: cached,
    cacheHitRatio: cached != null && evaluated != null && cached + evaluated > 0 ? cached / (cached + evaluated) : null,
    kvRestore: restore?.payload?.outcome ?? null,
    wallMs: observed.wallMs ?? null,
  };
}

function workspaceFile(workspace, path) {
  const root = resolve(workspace);
  const target = resolve(root, path);
  if (target !== root && !target.startsWith(root + sep)) throw new Error(`Assertion path escapes the workspace: ${path}`);
  return target;
}

/**
 * Evaluate task assertions. `readFile(absolutePath)` returns text or null when
 * the file does not exist; it is injected so tests need no filesystem.
 */
export function evaluateAssertions(assertions, { answer, metrics, workspace, readFile }) {
  const text = answer ?? '';
  return assertions.map(assertion => {
    const outcome = check(assertion);
    // `negate` inverts an answer check, e.g. "must not claim it wrote the file".
    const pass = assertion.negate ? !outcome.pass : outcome.pass;
    return { ...assertion, pass, detail: assertion.negate ? `negated: ${outcome.detail}` : outcome.detail };
  });

  function check(assertion) {
    switch (assertion.type) {
      case 'answerIncludes': {
        const haystack = assertion.caseSensitive ? text : text.toLowerCase();
        const needles = [assertion.text].flat().map(needle => assertion.caseSensitive ? needle : needle.toLowerCase());
        const missing = needles.filter(needle => !haystack.includes(needle));
        return { pass: missing.length === 0, detail: missing.length ? `missing: ${missing.join(', ')}` : 'found' };
      }
      case 'answerMatches': {
        const pass = new RegExp(assertion.pattern, assertion.flags ?? '').test(text);
        return { pass, detail: pass ? 'matched' : 'no match' };
      }
      case 'fileIncludes':
      case 'fileExists':
      case 'fileAbsent': {
        const content = readFile(workspaceFile(workspace, assertion.path));
        if (assertion.type === 'fileExists') return { pass: content != null, detail: content != null ? 'exists' : 'missing' };
        if (assertion.type === 'fileAbsent') return { pass: content == null, detail: content == null ? 'absent' : 'exists' };
        if (content == null) return { pass: false, detail: 'file missing' };
        const missing = [assertion.text].flat().filter(needle => !content.includes(needle));
        return { pass: missing.length === 0, detail: missing.length ? `missing: ${missing.join(', ')}` : 'found' };
      }
      case 'toolCalled': {
        const count = metrics.toolCallsByName[assertion.name] ?? 0;
        const min = assertion.min ?? 1;
        return { pass: count >= min, detail: `${count} call(s)` };
      }
      case 'toolNotCalled': {
        const count = metrics.toolCallsByName[assertion.name] ?? 0;
        return { pass: count === 0, detail: `${count} call(s)` };
      }
      case 'maxToolCalls':
        return { pass: metrics.toolCalls <= assertion.max, detail: `${metrics.toolCalls} call(s)` };
      case 'status':
        return { pass: metrics.status === assertion.equals, detail: metrics.status };
      default:
        throw new Error(`Unknown assertion type: ${assertion.type}`);
    }
  }
}

/** Validate a task definition before any model time is spent on it. */
export function validateTask(task, file) {
  const problems = [];
  if (typeof task.id !== 'string' || !/^[a-z0-9][a-z0-9-]*$/.test(task.id)) problems.push('id must be kebab-case');
  if (typeof task.prompt !== 'string' || !task.prompt.trim()) problems.push('prompt is required');
  if (!Array.isArray(task.assertions) || !task.assertions.length) problems.push('at least one assertion is required');
  if (task.approve !== undefined && task.approve !== 'all' && task.approve !== 'none' && !Array.isArray(task.approve)) problems.push('approve must be "all", "none" or a list of tool names');
  if (task.presets !== undefined && (!Array.isArray(task.presets) || !task.presets.length)) problems.push('presets must be a non-empty list when given');
  if (task.fixture !== undefined && (typeof task.fixture !== 'string' || !/^[a-z0-9][a-z0-9-]*$/.test(task.fixture))) problems.push('fixture must be a fixture directory name');
  const known = ['answerIncludes', 'answerMatches', 'fileIncludes', 'fileExists', 'fileAbsent', 'toolCalled', 'toolNotCalled', 'maxToolCalls', 'status'];
  for (const assertion of task.assertions ?? []) {
    if (!known.includes(assertion.type)) problems.push(`unknown assertion type ${assertion.type}`);
    if (assertion.negate && !['answerIncludes', 'answerMatches'].includes(assertion.type)) problems.push(`negate is only supported on answer assertions, not ${assertion.type}`);
    if (assertion.path && (assertion.path.includes('..') || /^[a-zA-Z]:|^[\\/]/.test(assertion.path))) problems.push(`assertion path must be workspace-relative: ${assertion.path}`);
  }
  if (problems.length) throw new Error(`${file}: ${problems.join('; ')}`);
  return task;
}
