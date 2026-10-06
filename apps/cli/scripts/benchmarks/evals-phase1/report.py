#!/usr/bin/env python3
"""MP-08 / MP-10: diagnostic CSV/plot; missing runs never become scores or free work."""
import argparse
import csv
import json
from pathlib import Path

from contract import validate_lock


def main():
    parser = argparse.ArgumentParser(description='MP-08 / MP-10: phase-1 diagnostic evidence')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.output.is_absolute():
        parser.error('absolute external evidence directory required')
    args.output.mkdir(parents=True, exist_ok=True)
    lock = json.loads(Path(__file__).with_name('inputs.lock.json').read_text())
    validate_lock(lock)
    rows = []
    for benchmark in ['terminal_bench_2', 'swe_verified_mini']:
        for task in lock[benchmark]['task_ids']:
            rows.append({'benchmark': benchmark, 'task_id': task, 'provider': 'codex',
                         'status': 'not_run_blocked', 'resolved': '', 'input_tokens': '',
                         'cached_input_tokens': '', 'output_tokens': '', 'reasoning_tokens': '',
                         'api_equivalent_usd': '', 'wall_time_seconds': ''})
    with (args.output / 'phase1-task-status.csv').open('x', newline='') as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]))
        writer.writeheader(); writer.writerows(rows)
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    fig, ax = plt.subplots(figsize=(7, 4))
    ax.set(xlabel='API-equivalent cost per task (USD)', ylabel='Task accuracy (%)',
           ylim=(0, 100), title='MP-08 / MP-10: Chariox phase 1 — no scored runs')
    ax.text(0.5, 0.5, '139 tasks pinned; 0 executed\nAccounting protocol allocation and acct-686 mapping pending',
            transform=ax.transAxes, ha='center', va='center', fontsize=10)
    ax.grid(alpha=0.2)
    fig.tight_layout()
    fig.savefig(args.output / 'phase1-cost-vs-accuracy.png', dpi=160)
    plt.close(fig)
    print('MP-08 / MP-10: diagnostic CSV and empty-data plot written; no accuracy/frontier claim')


if __name__ == '__main__':
    main()
