#!/usr/bin/env python3
"""Validate and summarize externally collected artifact evaluations; never invent runs."""
import argparse
import json
from pathlib import Path

DOMAINS = ('website', 'three_d', 'code')
STATUSES = ('passed', 'failed', 'blocked', 'not_checked', 'waived')


def summarize(records):
    groups = {}
    for record in records:
        for field in ('taskId', 'variant', 'model', 'sourceRevision', 'runId'):
            if not isinstance(record.get(field), str) or not record[field].strip():
                raise ValueError('Missing identity: ' + field)
        domain = record.get('domain')
        if domain not in DOMAINS:
            raise ValueError('Unknown domain')
        identity = (record['variant'], record['taskId'], record['runId'])
        if identity in groups:
            raise ValueError('Duplicate run')
        criteria = record.get('criteria')
        if not isinstance(criteria, list) or not criteria:
            raise ValueError('Run needs acceptance criteria')
        seen = set()
        for criterion in criteria:
            cid = criterion.get('id')
            if not isinstance(cid, str) or not cid.strip() or cid in seen:
                raise ValueError('Invalid or duplicate criterion ID')
            seen.add(cid)
            if criterion.get('status') not in STATUSES:
                raise ValueError('Invalid criterion status')
            if criterion['status'] == 'passed':
                evidence = criterion.get('evidence')
                if not isinstance(evidence, list) or not evidence or not all(isinstance(e, str) and e.strip() for e in evidence):
                    raise ValueError('Passing criterion needs evidence references')
                if criterion.get('sourceRevision') != record['sourceRevision']:
                    raise ValueError('Passing evidence is stale or unbound')
            elif not criterion.get('reason') and criterion['status'] in ('blocked', 'waived'):
                raise ValueError('Blocked/waived criterion needs a reason')
        groups[identity] = record
    summary = {}
    for record in groups.values():
        key = record['variant'] + '/' + record['domain']
        result = summary.setdefault(key, dict(runs=0, completed=0, blocked=0, failed=0, unverified=0))
        result['runs'] += 1
        statuses = [c['status'] for c in record['criteria']]
        result['completed'] += all(s == 'passed' for s in statuses)
        result['blocked'] += 'blocked' in statuses
        result['failed'] += 'failed' in statuses
        result['unverified'] += any(s in ('not_checked', 'waived') for s in statuses)
    return summary


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('records', type=Path, help='JSON array of collected runs')
    args = parser.parse_args()
    try:
        records = json.loads(args.records.read_text())
        if not isinstance(records, list):
            raise ValueError('Expected an array')
        print(json.dumps(summarize(records), indent=2))
    except (ValueError, KeyError, TypeError, OSError) as error:
        parser.exit(1, str(error) + '\n')
