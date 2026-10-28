#!/usr/bin/env python3
"""Report recorded synthetic trials without treating unfinished evidence as success.

Run after (or during) harness-evaluation.py. --regrade refreshes objective
checks using the current grader; request, response and workspace-change records
remain untouched. Human source/visual reviews belong in each trial's review.json.
"""
import argparse
import collections
import importlib.util
import json
import re
import sqlite3
import statistics
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
QA = ROOT / '.local/qa/harness-simplification'
OUT = QA / 'evaluations'


def read(path, default=None):
    return json.loads(path.read_text()) if path.exists() else default


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--regrade', action='store_true')
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('harness_grader', ROOT / 'scripts/harness-evaluation.py')
    grader = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(grader)
    tasks = {task['id']: task for task in grader.TASKS}
    rows = []
    for path in sorted(OUT.glob('*-v2/grade.json')):
        trial = path.parent
        grade = read(path)
        task = tasks[grade['task']]
        if args.regrade:
            grade.update(grader.grade(task, trial / 'workspace'))
            path.write_text(json.dumps(grade, indent=2))
        result = read(trial / 'result.json', {})
        review = read(trial / 'review.json', {})
        config = read(trial / 'configuration.json', {})
        run = result.get('run') or {}
        tools = []
        for message in result.get('messages', []):
            if message['role'] == 'tool':
                try:
                    tools.append(json.loads(message['content']))
                except (ValueError, TypeError):
                    pass
        failures = collections.Counter()
        signatures = collections.Counter()
        for tool in tools:
            request, value = tool.get('request', {}), tool.get('result', {})
            if not isinstance(value, dict):
                continue
            error = value.get('error')
            if isinstance(error, dict):
                failures[error.get('code', 'unknown')] += 1
            if value.get('status') == 'success' and request.get('name') not in ('process_poll', 'check_report'):
                signatures[json.dumps([request.get('name'), request.get('arguments')], sort_keys=True)] += 1
        messages = [m['content'] for m in result.get('messages', []) if m['role'] == 'assistant']
        answer = messages[-1] if messages else ''
        passed = bool(grade.get('passed'))
        # Structural games/SVGs and research answers need additional review.
        quality = False if not passed else review.get('passed', passed if task['grade'] in ('intervals', 'lru', 'csv') else None)
        completed = quality is True and not result.get('error') and run.get('status') == 'completed'
        catalog = config.get('catalog', [])
        rows.append({
            'identity': trial.name, 'task': task['id'], 'workload': task['workload'],
            'build': grade['build'], 'pool': grade['pool'], 'trial': grade['trial'],
            'revision': grade.get('revision'),
            'heldOut': task['heldOut'], 'evaluationIncomplete':grade.get('evaluationIncomplete',False), 'structuralPass': passed, 'reviewedPass': quality,
            'completed': completed, 'error': result.get('error'), 'status': run.get('status'),
            'outcome': run.get('outcome'), 'latencySeconds': grade.get('elapsedSeconds'),
            'toolCalls': len(tools), 'invalidArguments': failures['invalid-arguments'],
            'toolFailures': dict(failures), 'providerRequests': len(list(trial.glob('*-request.json'))),
            'repeatedSuccessfulSignatures': sum(max(0, n-1) for n in signatures.values()),
            'completionClaimRequiresReview': not passed and bool(re.search(r'\b(?:complete|completed|created|implemented|all done|verified)\b', answer, re.I)),
            'completedWithFailedOutcome': run.get('status') == 'completed' and quality is False,
            'evaluationBudgetStopped': True if (trial / 'budget-denials.jsonl').exists() else (None if 'rate limit or quota' in (result.get('error') or '').lower() else False),
            'falseCompletionClaim': review.get('falseCompletionClaim'),
            'factualErrors': review.get('factualErrors'),
            'factualReviewApplicable':task['grade'] in ('research','brief','schedule','data'),
            'catalogTools': len(catalog), 'catalogBytes': len(json.dumps(catalog, ensure_ascii=False, separators=(',', ':')).encode()),
            'buildIdentity': result.get('buildIdentity'), 'review': review,
            'outsideWorkspaceChanges': read(trial / 'outside-workspace-changes.json'),
        })
    incomplete = [row for row in rows if row['evaluationIncomplete']]
    for path in sorted(OUT.glob('*-v2/incomplete-evidence.json')):
        if (path.parent / 'grade.json').exists():
            continue
        incomplete.append({'identity':path.parent.name, **read(path), 'evaluationIncomplete':True})
    rows = [row for row in rows if not row['evaluationIncomplete']]
    followups = [row for row in rows if row['revision']]
    rows = [row for row in rows if not row['revision']]
    tokens = []
    with sqlite3.connect(OUT / 'tokens.sqlite') as db:
        for pool, count, charged, unsettled in db.execute('SELECT pool,count(*),sum(coalesce(charged,reserved)),sum(charged IS NULL) FROM requests GROUP BY pool'):
            tokens.append({'pool': pool, 'requests': count, 'chargedTokens': charged, 'unsettledReservations': unsettled})
    groups = []
    for build in ('baseline', 'candidate'):
        for pool in ('remote', 'local'):
            for workload in ('coding', 'research', 'document_visual'):
                values = [r for r in rows if (r['build'], r['pool'], r['workload']) == (build, pool, workload)]
                times = [v['latencySeconds'] for v in values if v['latencySeconds'] is not None]
                groups.append({'build': build, 'pool': pool, 'workload': workload, 'trials': len(values), 'expected': 12,
                    'structuralPasses': sum(v['structuralPass'] for v in values), 'reviewedCompletions': sum(v['completed'] for v in values),
                    'reviewsPending': sum(v['reviewedPass'] is None for v in values),
                    'interruptedOrFailed': sum(v['status'] != 'completed' for v in values),
                    'invalidArguments': sum(v['invalidArguments'] for v in values),
                    'toolCalls': sum(v['toolCalls'] for v in values),
                    'completedWithFailedOutcome': sum(v['completedWithFailedOutcome'] for v in values),
                    'documentedEvaluationBudgetStops': sum(v['evaluationBudgetStopped'] is True for v in values),
                    'quotaStopAttributionsPending': sum(v['evaluationBudgetStopped'] is None for v in values),
                    'reviewedFalseCompletionClaims': sum(v['falseCompletionClaim'] is True for v in values),
                    'falseCompletionReviewsPending': sum(v['falseCompletionClaim'] is None for v in values),
                    'factualAccuracyReviewsPending':sum(v['factualReviewApplicable'] and v['factualErrors'] is None for v in values),
                    'reviewedFactualErrors': sum(v['factualErrors'] or 0 for v in values),
                    'medianLatencySeconds': statistics.median(times) if times else None})
    revision_groups = []
    for revision, pool, workload in sorted({(row['revision'], row['pool'], row['workload']) for row in followups}):
        values = [row for row in followups if (row['revision'], row['pool'], row['workload']) == (revision, pool, workload)]
        revision_groups.append({'revision': revision, 'pool': pool, 'workload': workload, 'trials': len(values), 'expected': 12,
            'structuralPasses': sum(row['structuralPass'] for row in values), 'reviewedCompletions': sum(row['completed'] for row in values),
            'reviewsPending': sum(row['reviewedPass'] is None for row in values), 'toolCalls': sum(row['toolCalls'] for row in values),
            'invalidArguments': sum(row['invalidArguments'] for row in values),
            'interruptedOrFailed': sum(row['status'] != 'completed' for row in values),
            'reviewedFactualErrors': sum(row['factualErrors'] or 0 for row in values),
            'falseCompletionReviewsPending':sum(row['falseCompletionClaim'] is None for row in values),
            'factualAccuracyReviewsPending':sum(row['factualReviewApplicable'] and row['factualErrors'] is None for row in values)})
    revisions = [path for path in QA.glob('candidate-v*/manifest.json')
        if re.fullmatch(r'candidate-v\d+', path.parent.name)]
    latest = max(revisions, key=lambda path:int(path.parent.name.split('-v')[-1])) if revisions else None
    latest_revision = latest.parent.name.removeprefix('candidate-') if latest else None
    current = [row for row in followups if row['revision'] == latest_revision]
    report = {'expectedTrials': 144, 'recordedTrials': len(rows), 'liveEvidenceComplete': len(rows) == 144,
        'currentCandidateRevision':latest_revision, 'currentCandidateRecordedTrials':len(current),
        'currentCandidateExpectedTrials':72, 'currentCandidateLiveEvidenceComplete':len(current)==72,
        'currentCandidateManifest':{key:value for key,value in read(latest,{}).items() if key!='files'} if latest else None,
        'promotionApproved': False, 'incompleteEvidence':incomplete, 'tokens': tokens, 'groups': groups, 'trials': rows, 'followupTrials': followups,
        'revisionGroups': revision_groups,
        'sourceReduction': read(QA / 'source-reduction-v7.json'),
        'workingTreeSourceReduction': read(QA / 'source-reduction.json'), 'dependencyReduction': read(QA / 'dependency-reduction.json'),
        'baselineManifest': read(QA / 'baseline/manifest.json'), 'candidateManifest': read(QA / 'candidate/manifest.json')}
    (OUT / 'comparison.json').write_text(json.dumps(report, indent=2))
    lines = ['# Harness comparison — recorded evidence', '',
        f"Recorded {len(rows)} of 144 planned trials. **These measurements do not yet justify replacing the baseline.**", '',
        'These candidate trials use the frozen V7 binary. Subsequent working-tree fixes have separate identities and are not credited to these trials.', '',
        'Structural passes do not establish source accuracy, visual quality, or gameplay. Reviewed completion also requires a successful terminal run. Unfinished trials remain incomplete.', '',
        f'Latest frozen candidate: **{latest_revision}**, with **{len(current)} of 72** remote/local trials recorded. Its evidence is separate from the earlier comparison.', '',
        '| Build | Model pool | Workload | Trials / 12 | Structural passes | Reviewed completions | Reviews pending |',
        '|---|---|---|---:|---:|---:|---:|']
    for g in groups:
        lines.append(f"| {g['build']} | {g['pool']} | {g['workload']} | {g['trials']} | {g['structuralPasses']} | {g['reviewedCompletions']} | {g['reviewsPending']} |")
    if followups:
        lines += ['', 'Later frozen revisions are separate from the 144-trial V7 comparison. Each revision/model/workload needs all 12 trials and their reviews before a full comparison is established.', '',
            '| Revision | Pool | Workload | Trials / 12 | Structural passes | Reviewed completions | Reviews pending |',
            '|---|---|---|---:|---:|---:|---:|']
        for group in revision_groups:
            lines.append(f"| {group['revision']} | {group['pool']} | {group['workload']} | {group['trials']} | {group['structuralPasses']} | {group['reviewedCompletions']} | {group['reviewsPending']} |")
        lines += ['', 'Per-trial revision records:', '',
            '| Revision | Pool | Task | Trial | Structural pass | Reviewed completion |',
            '|---|---|---|---:|---|---|']
        for row in followups:
            lines.append(f"| {row['revision']} | {row['pool']} | {row['task']} | {row['trial']} | {row['structuralPass']} | {row['completed']} |")
    if incomplete:
        lines += ['', f"{len(incomplete)} additional trial(s) have incomplete evidence, from a combined-cap stop or an explicitly recorded evaluation interruption. They are excluded from quality denominators; their artifacts and charges remain recorded.", '']
    lines += ['', 'Remote pool: configured DeepSeek-V4.1-Flash-1017 through River AI. Local pool: installed Gemma-4-E4B Q2_K_XL.', '',
        'Each trial has a 600-second wall budget, 250,000-token allowance, 32,768-token context and 4,096 output tokens per request. The shared remote cap is 20,000,000 input-plus-output tokens, including excluded diagnostic attempts. Missing usage consumes the full reservation. Local tokens are counted separately.', '']
    for token in tokens:
        lines.append(f"- {token['pool']}: {token['chargedTokens']:,} charged/reserved tokens across {token['requests']} requests; {token['unsettledReservations']} unsettled reservations conservatively charged.")
    lines += ['', 'Evaluation budget denials are explicit fixture stops and are not evidence of an upstream provider outage. False completion and factual-accuracy dimensions require individual review; pending counts remain explicit in comparison.json. A completed run with a failed deliverable is measured separately. Duplicate successful request signatures are a proxy for unnecessary calls, not proof that every repeat was unnecessary.', '']
    lines += ['', 'The Gemma runtime uses the recorded equivalent inline-loop adaptation of its installed unsupported template filter. Both builds use the same server and template. Trials are serialized on that server. Builds and tests overlapped some trials; latency is descriptive and is not a controlled performance result.', '',
        'Evaluation workspaces are synthetic directories under the repository. Both builds therefore inherited the same ancestor AGENTS.md; captured request files record that context. Full Access commands have no OS filesystem sandbox. One baseline research trial copied two scratch files into the repository root; their tool-audit provenance and hashes are recorded in outside-workspace-changes.json and the files are preserved with that trial. Actual user conversation storage is read-only.', '',
        'Excluded preliminary fixture/runtime attempts and the first diagnostic candidate are retained in the evaluation directory. Their remote charges remain within the shared cap. Source/visual reviews must record source URLs, factual omissions and limitations in review.json; an agent’s final paragraph is never sufficient evidence.', '',
        'Reproduce/update: `python3 scripts/harness-evaluation-report.py --regrade`. Detailed measurements and exact build hashes: `.local/qa/harness-simplification/evaluations/comparison.json`.', '']
    (ROOT / 'evaluations/harness/RESULTS.md').write_text('\n'.join(lines))
    print(json.dumps({'recordedTrials': len(rows), 'tokens': tokens, 'promotionApproved': False}))


if __name__ == '__main__':
    main()
