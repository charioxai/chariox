#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: report actual campaigns without inventing missing costs."""
import argparse
import csv
import json
from decimal import Decimal
from pathlib import Path

from contract import validate_lock
from proxy_prices import proxy_quote


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--campaign', type=Path, required=True)
    parser.add_argument('--score-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    source_root = Path(__file__).resolve().parents[5]
    if not args.output.is_absolute() or args.output.resolve().is_relative_to(source_root):
        parser.error('MP-11: absolute external evidence directory required')
    lock = json.loads(Path(__file__).with_name('inputs.lock.json').read_text())
    validate_lock(lock)
    campaign = json.loads((args.campaign / 'campaign.json').read_text())
    scoring = json.loads((args.score_root / 'scoring-command.json').read_text())
    expected = lock['swe_verified_mini']['task_ids'][:10] if campaign['phase'] == 'smoke' else lock['swe_verified_mini']['task_ids']
    if campaign['task_ids'] != expected or scoring['task_ids'] != expected:
        raise ValueError('MP-08 / MP-10: campaign/scorer denominator mismatch')
    if scoring['harness_revision'] != lock['swe_bench_harness']['revision']:
        raise ValueError('MP-08 / MP-10: scorer revision mismatch')
    reports = list(args.score_root.glob('chariox-*.' + scoring['run_id'] + '.json'))
    if len(reports) != 1:
        raise ValueError('MP-08 / MP-10: exactly one official aggregate report required')
    official = json.loads(reports[0].read_text())
    if sorted(official['submitted_ids']) != expected or official['total_instances'] != len(expected):
        raise ValueError('MP-08 / MP-10: official report denominator mismatch')
    resolved = set(official['resolved_ids'])
    rows = []
    for task in expected:
        result = json.loads((args.campaign / (task + '.json')).read_text())
        if result['source_commit'] != campaign['source_commit'] or result['kernel_sha256'] != campaign['kernel_sha256']:
            raise ValueError('MP-08 / MP-10: task source identity mismatch')
        usage = result.get('usage') or {}
        cost = usage.get('api_equivalent_nanodollars')
        proxy = proxy_quote(campaign['model'], usage) if usage else None
        rows.append({'benchmark': 'swe_verified_mini', 'phase': campaign['phase'], 'task_id': task,
                     'provider': 'codex', 'model': campaign['model'], 'status': result['status'],
                     'official_outcome': 'error' if task in official['error_ids'] else 'empty_patch' if task in official['empty_patch_ids'] else 'resolved' if task in resolved else 'unresolved',
                     'resolved': task in resolved, 'cleanup_complete': result['cleanup_complete'],
                     **{key: usage.get(key, '') for key in ['input_tokens', 'cached_input_tokens', 'output_tokens', 'reasoning_tokens']},
                     'api_equivalent_usd': '' if cost is None else str(Decimal(cost) / Decimal(10**9)),
                     'proxy_usd_lower': '' if proxy is None else str(Decimal(proxy['lower_nanodollars']) / Decimal(10**9)),
                     'proxy_usd_upper': '' if proxy is None else str(Decimal(proxy['upper_nanodollars']) / Decimal(10**9)),
                     'proxy_unknown_fields': '' if proxy is None else ','.join(proxy['unknown_fields']),
                     'wall_time_seconds': result['wall_time_seconds']})
    args.output.mkdir(parents=True, exist_ok=True)
    with (args.output / 'task-results.csv').open('x', newline='') as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]))
        writer.writeheader(); writer.writerows(rows)
    summary = {'mp_items': ['MP-08', 'MP-10', 'MP-11'], 'phase': campaign['phase'],
               'source_commit': campaign['source_commit'], 'kernel_sha256': campaign['kernel_sha256'],
               'harness_revision': scoring['harness_revision'], 'official_run_id': scoring['run_id'],
               'denominator': len(expected), 'resolved': len(resolved),
               'accuracy_percent': 100 * len(resolved) / len(expected),
               'scorer_errors': official['error_instances'],
               'cleanup_complete': all(row['cleanup_complete'] for row in rows),
               'priced_tasks': sum(row['api_equivalent_usd'] != '' for row in rows),
               'wall_time_seconds': sum(row['wall_time_seconds'] for row in rows),
               'scope': 'local reproduction using unchanged official scorer; smoke is not full acceptance'}
    summary['tokens'] = {key: sum(row[key] for row in rows) if all(type(row[key]) is int for row in rows) else None for key in ['input_tokens','cached_input_tokens','output_tokens','reasoning_tokens']}
    summary['proxy_cost'] = {'label':'proxy','lower_usd': str(sum(Decimal(r['proxy_usd_lower']) for r in rows)), 'upper_usd': str(sum(Decimal(r['proxy_usd_upper']) for r in rows)), 'unknown_fields':['context_band','cache_write_tokens']} if all(r['proxy_usd_lower'] != '' for r in rows) else None
    summary['proxy_mapping'] = json.loads(Path(__file__).with_name('proxy-prices-2026-10-06.json').read_text())
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    fig, ax = plt.subplots(figsize=(7, 4))
    ax.set(xlabel='Proxy API cost per task (USD; bounded)', ylabel='Task accuracy (%)',
           ylim=(0, 100), title='MP-08 / MP-10: SWE Verified Mini ' + campaign['phase'])
    if summary['proxy_cost']:
        lower = float(summary['proxy_cost']['lower_usd']) / len(rows)
        upper = float(summary['proxy_cost']['upper_usd']) / len(rows)
        ax.plot([lower,upper],[summary['accuracy_percent']]*2,marker='|',linewidth=3,label='proxy interval; band/cache-write unknown')
        ax.annotate(f"{len(resolved)}/{len(expected)}",((lower+upper)/2,summary['accuracy_percent']),xytext=(0,8),textcoords='offset points',ha='center')
        ax.legend(loc='lower right')
    else:
        ax.text(0.5,0.5,'Proxy unavailable; no cost/accuracy point',transform=ax.transAxes,ha='center')
    ax.grid(alpha=0.2); fig.tight_layout()
    fig.savefig(args.output / 'cost-vs-accuracy.png', dpi=160)
    plt.close(fig)
    print('MP-08 / MP-10: actual task CSV and scoped summary written')


if __name__ == '__main__':
    main()
