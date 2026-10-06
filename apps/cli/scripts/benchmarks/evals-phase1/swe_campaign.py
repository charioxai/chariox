#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: serial, resumable official Mini solver campaign."""
import argparse
import hashlib
import json
import subprocess
import sys
import time
from uuid import uuid4
from pathlib import Path

from swe_adapter import load_tasks
from turn import preflight, resource_sample



def archive_quota_attempt(result, prediction):
    """MP-08 / MP-10 / MP-11: fresh retry rechecks product quota; keep prior evidence."""
    if not result.exists():return False
    measurement=json.loads(result.read_text())
    if measurement.get('status')!='quota_exhausted':return False
    if not measurement.get('cleanup_complete'):
        raise RuntimeError('MP-11: quota attempt cleanup incomplete')
    if prediction.exists():
        raise RuntimeError('MP-08 / MP-10: quota admission unexpectedly produced a prediction')
    archive=result.parent/'quota-attempts'/uuid4().hex
    archive.mkdir(parents=True,mode=0o700)
    evidence=result.with_suffix('.evidence')
    if evidence.exists():evidence.rename(archive/evidence.name)
    log=result.with_suffix('.runner.log')
    if log.exists():log.rename(archive/log.name)
    result.rename(archive/result.name)
    return True

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime-root', type=Path, required=True)
    parser.add_argument('--parquet', type=Path, required=True)
    parser.add_argument('--profile-path', required=True)
    parser.add_argument('--model', required=True)
    parser.add_argument('--local-protocol', type=int, required=True)
    parser.add_argument('--workspace-root', type=Path, required=True)
    parser.add_argument('--output-root', type=Path, required=True)
    parser.add_argument('--smoke', action='store_true')
    args = parser.parse_args()
    source_root = Path(__file__).resolve().parents[5]
    for path in [args.output_root, args.workspace_root]:
        if not path.is_absolute() or path.resolve().is_relative_to(source_root):
            parser.error('MP-11: absolute external task/output roots required')
    manifest = json.loads((args.runtime_root / 'eval-runtime.json').read_text())
    preflight(str(args.runtime_root), manifest['source_commit'], manifest['kernel_sha256'], args.local_protocol)
    tasks = load_tasks(args.parquet)
    ids = sorted(tasks)[:10] if args.smoke else sorted(tasks)
    receipt = {'mp_items': ['MP-08', 'MP-10', 'MP-11'], 'phase': 'smoke' if args.smoke else 'full',
               'task_ids': ids, 'source_commit': manifest['source_commit'],
               'kernel_sha256': manifest['kernel_sha256'], 'model': args.model, 'effort': 'low',
               'local_protocol': args.local_protocol,
               'campaign_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    args.output_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    campaign = args.output_root / 'campaign.json'
    if campaign.exists():
        previous=json.loads(campaign.read_text())
        if {k:v for k,v in previous.items() if k!='campaign_sha256'} != {k:v for k,v in receipt.items() if k!='campaign_sha256'}:
            parser.error('MP-08 / MP-10: existing campaign identity differs')
    else:
        campaign.write_text(json.dumps(receipt, indent=2) + '\n')
    attempts=args.output_root/'campaign-attempts'
    attempts.mkdir(mode=0o700,exist_ok=True)
    (attempts/(uuid4().hex+'.json')).write_text(json.dumps(dict(receipt,started_at=time.time()),indent=2)+'\n')
    args.workspace_root.mkdir(parents=True, exist_ok=True)
    for instance in ids:
        before = resource_sample()
        task = tasks[instance]
        workspace = args.workspace_root / instance
        result = args.output_root / (instance + '.json')
        prediction = args.output_root / (instance + '.prediction.jsonl')
        archive_quota_attempt(result, prediction)
        if not result.exists():
            if not workspace.exists():
                subprocess.run(['git', 'init', str(workspace)], check=True, stdout=subprocess.DEVNULL)
                with (args.output_root / (instance + '.checkout.log')).open('w') as log:
                    subprocess.run(['git', '-C', str(workspace), 'fetch', '--depth', '1',
                                    'https://github.com/' + task['repo'] + '.git', task['base_commit']],
                                   check=True, stdout=log, stderr=log)
                    subprocess.run(['git', '-C', str(workspace), 'checkout', '--detach', 'FETCH_HEAD'],
                                   check=True, stdout=log, stderr=log)
            command = [sys.executable, str(Path(__file__).with_name('swe_adapter.py')),
                       '--parquet', str(args.parquet), '--instance-id', instance, '--workspace', str(workspace),
                       '--runtime-root', str(args.runtime_root), '--profile-path', args.profile_path,
                       '--source-commit', manifest['source_commit'], '--kernel-sha256', manifest['kernel_sha256'],
                       '--local-protocol', str(args.local_protocol), '--model', args.model,
                       '--result', str(result), '--prediction', str(prediction)]
            started = time.time()
            with (args.output_root / (instance + '.runner.log')).open('w') as log:
                code = subprocess.call(command, stdout=log, stderr=log)
            with (args.output_root / 'commands.jsonl').open('a') as log:
                log.write(json.dumps({'mp_items': receipt['mp_items'], 'instance_id': instance, 'command': command,
                                      'exit_code': code, 'started': started, 'resource_before': before,
                                      'resource_after': resource_sample()}) + '\n')
        if not result.exists():
            raise RuntimeError('MP-08 / MP-10: runner admission blocked at ' + instance)
        measurement = json.loads(result.read_text())
        print('MP-08 / MP-10: ' + instance + ' ' + measurement['status'], flush=True)
        if measurement['status'] == 'quota_exhausted':
            return 2  # Keep this attempt; retry only after a fresh plan window.
        if not measurement['cleanup_complete']:
            raise RuntimeError('MP-11: incomplete runtime cleanup at ' + instance)
        if not prediction.exists():
            # Settled failures stay in the official denominator, with no patch.
            prediction.write_text(json.dumps({'instance_id': instance,
                                  'model_name_or_path': 'chariox-' + manifest['source_commit'], 'model_patch': ''}) + '\n')
    (args.output_root / 'predictions.jsonl').write_text(''.join(
        (args.output_root / (i + '.prediction.jsonl')).read_text() for i in ids))
    print('MP-08 / MP-10: exact ' + str(len(ids)) + ' predictions ready for official scoring', flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
