#!/usr/bin/env python3
"""MP-08 / MP-10: pin inputs then invoke the unchanged official Docker evaluator."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import re
from uuid import uuid4

from swe_adapter import load_tasks


def main():
    parser = argparse.ArgumentParser(description='MP-08 / MP-10: official HAL Mini scoring')
    parser.add_argument('--harness-root', type=Path, required=True)
    parser.add_argument('--parquet', type=Path, required=True)
    parser.add_argument('--predictions', type=Path, required=True)
    parser.add_argument('--output-root', type=Path, required=True)
    parser.add_argument('--run-id', required=True)
    parser.add_argument('--smoke', action='store_true')
    args = parser.parse_args()
    lock = json.loads(Path(__file__).with_name('inputs.lock.json').read_text())
    actual = subprocess.check_output(['git', '-C', str(args.harness_root), 'rev-parse', 'HEAD'], text=True).strip()
    dirty = subprocess.check_output(['git', '-C', str(args.harness_root), 'status', '--porcelain'], text=True).strip()
    if actual != lock['swe_bench_harness']['revision'] or dirty:
        raise ValueError('MP-08 / MP-10: official evaluator revision mismatch')
    tasks = load_tasks(args.parquet)
    ids = lock['swe_verified_mini']['task_ids'][:10] if args.smoke else lock['swe_verified_mini']['task_ids']
    predictions = [json.loads(line) for line in args.predictions.read_text().splitlines() if line.strip()]
    if sorted(p['instance_id'] for p in predictions) != sorted(ids):
        raise ValueError('MP-08 / MP-10: predictions must cover exact smoke/full denominator once')
    if not args.output_root.is_absolute() or args.output_root.resolve().is_relative_to(args.harness_root.resolve()):
        raise ValueError('MP-11: external evaluator output directory required')
    if not re.fullmatch(r'chariox-evals-[a-z0-9-]{1,32}', args.run_id):
        raise ValueError('MP-11: lane-owned evaluator run label required')
    # The official harness removes an existing same-name container. A fresh
    # nonce and preflight avoid handing it another run's container name.
    run_id = args.run_id + '-' + uuid4().hex
    names = subprocess.check_output(['docker', 'ps', '-a', '--format', '{{.Names}}'], text=True).splitlines()
    if any(name.startswith('sweb.eval.') and name.endswith('.' + run_id) for name in names):
        raise ValueError('MP-11: evaluator container namespace already occupied')
    args.output_root.mkdir(parents=True, exist_ok=False)
    # Local JSON dataset freezes the exact parquet revision, avoiding a mutable
    # HuggingFace main lookup in the official harness. Gold data stays scorer-only.
    dataset = args.output_root / 'dataset.json'
    dataset.write_text(json.dumps([tasks[i] for i in ids]))
    env = dict(os.environ, PYTHONPATH=str(args.harness_root.resolve()), PYTHONDONTWRITEBYTECODE='1')
    command = [os.sys.executable, '-m', 'swebench.harness.run_evaluation',
               '--dataset_name', str(dataset.resolve()), '--predictions_path', str(args.predictions.resolve()),
               '--max_workers', '1', '--run_id', run_id]
    receipt = {'mp_items': ['MP-08', 'MP-10'], 'harness_revision': actual,
               'task_ids': ids, 'run_id': run_id, 'smoke': args.smoke, 'command': command}
    (args.output_root / 'scoring-command.json').write_text(json.dumps(receipt, indent=2) + '\n')
    # Official evaluator owns its tagged resources; no Docker/system prune.
    return subprocess.call(command, cwd=args.output_root, env=env)


if __name__ == '__main__':
    raise SystemExit(main())
