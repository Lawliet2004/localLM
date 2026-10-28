#!/usr/bin/env python3
"""Compare handwritten production and test sources with the preserved dirty baseline.

Rust test-only item ranges are estimated lexically; this is not compiler coverage.
Read the original archive, rather than the evaluation bootstrap's modified copy.
"""
import argparse
import hashlib
import json
import re
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
QA = ROOT / '.local/qa/harness-simplification'
EXTENSIONS = {'.rs', '.ts', '.tsx', '.css'}


def rust_mask(text):
    """Hide literals/comments while preserving offsets and newlines for item braces."""
    out = list(text)
    i = 0
    while i < len(text):
        end = i
        if text.startswith('//', i):
            end = text.find('\n', i)
            if end < 0:
                end = len(text)
        elif text.startswith('/*', i):
            end, depth = i + 2, 1
            while end < len(text) and depth:
                if text.startswith('/*', end):
                    depth += 1
                    end += 2
                elif text.startswith('*/', end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
        else:
            raw = re.match(r'r(#+)?"', text[i:]) if text[i] == 'r' else None
            char = re.match(r"'(?:\\u\{[0-9a-fA-F_]+\}|\\.|[^'\\\n])'", text[i:]) if text[i] == "'" else None
            if raw:
                close = '"' + (raw[1] or '')
                at = text.find(close, i + raw.end())
                end = len(text) if at < 0 else at + len(close)
            elif char:
                end = i + char.end()
            elif text[i] == '"':
                end = i + 1
                while end < len(text):
                    if text[end] == '\\':
                        end += 2
                    elif text[end] == '"':
                        end += 1
                        break
                    else:
                        end += 1
        if end > i:
            for index in range(i, min(end, len(text))):
                if text[index] != '\n':
                    out[index] = ' '
            i = end
        else:
            i += 1
    return ''.join(out)


def count(name, text):
    path = Path(name)
    test_file = '.test.' in name or 'test' in path.parts or path.stem.endswith(('_tests', '_regressions')) or path.stem in ('test_support', 'presentation_tests')
    test_lines = set()
    if path.suffix == '.rs' and not test_file:
        mask = rust_mask(text)
        for match in re.finditer(r'#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]', mask):
            start = match.start()
            at = match.end()
            while at < len(mask) and mask[at] not in '{;':
                at += 1
            if at < len(mask) and mask[at] == '{':
                depth = 1
                at += 1
                while at < len(mask) and depth:
                    depth += (mask[at] == '{') - (mask[at] == '}')
                    at += 1
            first = text.count('\n', 0, start)
            last = text.count('\n', 0, at)
            test_lines.update(range(first, last + 1))
    result = dict(production=0, tests=0, productionBytes=0, testBytes=0)
    for number, line in enumerate(text.splitlines(keepends=True)):
        if not line.strip():
            continue
        test = test_file or number in test_lines
        result['tests' if test else 'production'] += 1
        result['testBytes' if test else 'productionBytes'] += len(line.encode())
    return result


def included(name):
    return (name.startswith('src/') or name.startswith('src-tauri/src/')) and Path(name).suffix in EXTENSIONS and '/generated/' not in name


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--candidate-archive', type=Path, help='Compare a frozen candidate source archive instead of the current tree')
    parser.add_argument('--output', type=Path, default=QA / 'source-reduction.json')
    args = parser.parse_args()
    def archived(path):
        with tarfile.open(path) as archive:
            return {f.name: archive.extractfile(f).read().decode() for f in archive if f.isfile() and included(f.name)}
    baseline = archived(QA / 'baseline/source.tar.gz')
    candidate = archived(args.candidate_archive) if args.candidate_archive else {str(p.relative_to(ROOT)): p.read_text() for root in ('src', 'src-tauri/src') for p in (ROOT / root).rglob('*') if p.is_file() and included(str(p.relative_to(ROOT)))}
    totals = {label: dict(production=0, tests=0, productionBytes=0, testBytes=0) for label in ('baseline', 'candidate')}
    files = []
    for name in sorted(baseline.keys() | candidate.keys()):
        before, after = count(name, baseline.get(name, '')), count(name, candidate.get(name, ''))
        for label, value in [('baseline', before), ('candidate', after)]:
            for key, number in value.items():
                totals[label][key] += number
        delta = {key: after[key] - before[key] for key in before}
        if any(delta.values()):
            files.append(dict(path=name, **delta))
    report = dict(method='Nonempty handwritten .rs/.ts/.tsx/.css source lines and their UTF-8 bytes in src/ and src-tauri/src/. Rust cfg(test) items and named *_tests, *_regressions, test_support modules are counted as tests, as are TS .test files and src/test. Generated assets, workers, docs, evaluator scripts and lockfiles excluded. Lexical classification is an estimate, not compiler coverage.', methodVersion=2,
        candidateBasis=str(args.candidate_archive) if args.candidate_archive else 'current working tree',
        baselineArchiveSha256=hashlib.sha256((QA / 'baseline/source.tar.gz').read_bytes()).hexdigest(),
        **totals, delta={k: totals['candidate'][k] - totals['baseline'][k] for k in totals['baseline']}, files=files)
    args.output.write_text(json.dumps(report, indent=2))
    print(json.dumps({k: v for k, v in report.items() if k != 'files'}))


if __name__ == '__main__':
    main()
