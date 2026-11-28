#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: HAL Mini predictions through a real Chariox TUI.

Run inside a disposable solver environment prepared at the pinned instance base.
Only the problem statement enters the provider, never solution or test patches.
The separate scoring command runs the official SWE-bench Docker evaluator.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

from contract import validate_lock
from turn import run


def load_tasks(parquet):
    import pyarrow.parquet as pq
    lock = json.loads(Path(__file__).with_name('inputs.lock.json').read_text())
    validate_lock(lock)
    pin = lock['swe_verified_mini']
    with parquet.open('rb') as source:
        digest = hashlib.file_digest(source, 'sha256').hexdigest()
    if digest != pin['parquet_sha256']:
        raise ValueError('MP-08 / MP-10: HAL dataset digest mismatch')
    rows = pq.read_table(parquet).to_pylist()
    if sorted(row['instance_id'] for row in rows) != pin['task_ids']:
        raise ValueError('MP-08 / MP-10: HAL task-set mismatch')
    return {row['instance_id']: row for row in rows}


def prediction(instance_id, task, patch, source_commit):
    if task['instance_id'] != instance_id:
        raise ValueError('MP-08 / MP-10: instance attribution mismatch')
    return {'instance_id': instance_id, 'model_name_or_path': f'chariox-{source_commit}', 'model_patch': patch}


def main():
    parser = argparse.ArgumentParser(description='MP-08 / MP-10 / MP-11: one HAL task through Chariox')
    parser.add_argument('--parquet', type=Path, required=True)
    parser.add_argument('--instance-id', required=True)
    parser.add_argument('--workspace', type=Path, required=True)
    parser.add_argument('--runtime-root', required=True)
    parser.add_argument('--profile-path')
    parser.add_argument('--placement-config', type=Path)
    parser.add_argument('--source-commit', required=True)
    parser.add_argument('--kernel-sha256', required=True)
    parser.add_argument('--local-protocol', type=int, required=True)
    parser.add_argument('--provider', default='codex', choices=['codex', 'claude', 'opencode'])
    parser.add_argument('--model', required=True)
    parser.add_argument('--result', type=Path, required=True)
    parser.add_argument('--prediction', type=Path, required=True)
    args = parser.parse_args()
    task = load_tasks(args.parquet)[args.instance_id]
    workspace = args.workspace.resolve(strict=True)
    for out in [args.result, args.prediction]:
        if not out.is_absolute() or out.resolve().is_relative_to(workspace) or out.exists():
            raise ValueError('MP-11: new external output path required')
    def git(*operands):
        return subprocess.check_output(['git', '-C', str(workspace), *operands], text=True)
    if git('rev-parse', 'HEAD').strip() != task['base_commit'] or git('status', '--porcelain').strip():
        raise ValueError('MP-08 / MP-10: solver checkout must be clean at pinned task base')
    request = {'instruction': task['problem_statement'], 'workspace': str(workspace),
               'runtime_root': args.runtime_root, 'profile_path': args.profile_path,
               'source_commit': args.source_commit, 'kernel_sha256': args.kernel_sha256,
               'local_protocol': args.local_protocol, 'provider': args.provider, 'model': args.model}
    if args.placement_config:
        config = json.loads(args.placement_config.read_text())
        allowed = {'placement', 'home_kernel_url', 'home_session_ref', 'home_agent_id', 'worker_kernel_id', 'home_auth_file'}
        if set(config) - allowed:
            raise ValueError('MP-11: placement config accepts references only, not credentials or arbitrary overrides')
        request.update(config)
    result = run(request, args.result.with_suffix('.evidence'))
    result['instance_id'] = args.instance_id
    with args.result.open('x') as output:
        output.write(json.dumps(result, indent=2) + '\n')
    if result['status'] != 'completed' or not result['cleanup_complete']:
        return 1
    # Include new source files, but refuse credential-like paths before diff reads.
    new_files = git('ls-files', '--others', '--exclude-standard', '-z').split('\0')
    changed = git('diff', '--name-only', '-z', 'HEAD').split('\0')
    for name in filter(None, new_files + changed):
        if Path(name).name in ['auth.json', '.credentials.json'] or re.search(r'(^|/)(\.env|\.codex|\.claude)(/|$)', name):
            raise ValueError('MP-11: refusing a credential-like prediction path')
        if name in new_files:
            subprocess.check_call(['git', '-C', str(workspace), 'add', '-N', '--', name], stdout=subprocess.DEVNULL)
    patch = git('diff', '--no-ext-diff', '--binary', 'HEAD')
    record = prediction(args.instance_id, task, patch, args.source_commit)
    with args.prediction.open('x') as output:
        output.write(json.dumps(record) + '\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
