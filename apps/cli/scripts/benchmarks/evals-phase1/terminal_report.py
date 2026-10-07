#!/usr/bin/env python3
"""MP-08 / MP-10: exact official Harbor outcomes, primary tokens and proxy bounds."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
from decimal import Decimal

from contract import validate_lock, admit_token_usage
from proxy_prices import proxy_quote, load_price_table


def report(campaign_dir, output):
    scripts=Path(__file__).resolve().parent
    lock=json.loads((scripts/'inputs.lock.json').read_text());validate_lock(lock)
    campaign=json.loads((campaign_dir/'campaign.json').read_text())
    expected=lock['terminal_bench_2']['task_ids'][:10] if campaign['phase']=='smoke' else lock['terminal_bench_2']['task_ids']
    if campaign['phase'] not in ['smoke','full'] or campaign['task_ids']!=expected or campaign['harness_revision']!=lock['harbor']['revision'] or campaign['task_revision']!=lock['terminal_bench_2']['revision']:
        raise ValueError('MP-08 / MP-10: campaign denominator/provenance mismatch')
    entries={item['task_id']:item for item in campaign['tasks']}
    if len(entries)!=len(campaign['tasks']) or not set(entries)<=set(expected):
        raise ValueError('MP-08 / MP-10: duplicate or foreign task')
    rows=[];solver_walls=[];tokens=['input_tokens','cached_input_tokens','output_tokens','reasoning_tokens']
    for task in expected:
        item=entries.get(task)
        row={'benchmark':'terminal_bench_2','phase':campaign['phase'],'task_id':task,'status':'not_run',
             'reward':'','official_exception':'','cleanup_complete':False,**{key:'' for key in tokens},
             'proxy_usd_lower':'','proxy_usd_upper':'','proxy_unknown_fields':'','wall_time_seconds':'','official_result_sha256':''}
        if item:
            path=Path(item['official_result']);official=json.loads(path.read_text())
            if official['task_name'].split('/')[-1]!=task or official['agent_info']['version']!=campaign['source_commit']:
                raise ValueError('MP-08 / MP-10: official task identity mismatch')
            metadata=((official.get('agent_result') or {}).get('metadata') or {}).get('chariox') or {}
            if metadata and (metadata['source_commit']!=campaign['source_commit'] or metadata['kernel_sha256']!=campaign['kernel_sha256']):
                raise ValueError('MP-08 / MP-10: runtime artifact mismatch')
            reward=((official.get('verifier_result') or {}).get('rewards') or {}).get('reward')
            exception=(official.get('exception_info') or {}).get('exception_type')
            row.update(status=metadata.get('status','harness_error'),reward='' if reward is None else reward,
                       official_exception=exception or '',cleanup_complete=bool(metadata.get('cleanup_complete')) and not item['cleanup']['remaining_containers'],
                       wall_time_seconds=item['wall_time_seconds'],official_result_sha256=hashlib.sha256(path.read_bytes()).hexdigest())
            if reward is not None:solver_walls.append(metadata.get('wall_time_seconds'))
            usage=metadata.get('usage')
            if usage:
                admit_token_usage(usage,metadata['session_id'])
                row.update({key:usage.get(key,'') for key in tokens})
                quote=proxy_quote(campaign['model'],usage)
                if quote:
                    row.update(proxy_usd_lower=str(Decimal(quote['lower_nanodollars'])/10**9),
                               proxy_usd_upper=str(Decimal(quote['upper_nanodollars'])/10**9),
                               proxy_unknown_fields=','.join(quote['unknown_fields']))
        rows.append(row)
    complete=campaign['status']=='completed' and len(entries)==len(expected) and all(r['reward']!='' and not r['official_exception'] and r['cleanup_complete'] for r in rows)
    successes=sum(r['reward']==1 for r in rows)
    summary={'mp_items':['MP-08','MP-10','MP-11'],'benchmark':'terminal_bench_2','phase':campaign['phase'],
             'source_commit':campaign['source_commit'],'kernel_sha256':campaign['kernel_sha256'],
             'harness_revision':campaign['harness_revision'],'task_revision':campaign['task_revision'],
             'denominator':len(expected),'attempted':len(entries),'scored':sum(r['reward']!='' for r in rows),
             'successes':successes,'complete':complete,'accuracy_percent':100*successes/len(expected) if complete else None,
             'harness_errors':sum(bool(r['official_exception']) for r in rows),'cleanup_complete':all(r['cleanup_complete'] for r in rows),
             'wall_time_seconds':campaign['finished_at']-campaign['started_at'],
             'solver_wall_time_seconds':sum(solver_walls) if all(type(w) in [int,float] for w in solver_walls) else None,
             'tokens':{key:sum(r[key] for r in rows) if all(type(r[key]) is int for r in rows) else None for key in tokens},
             'known_token_subtotal':{key:sum(r[key] for r in rows if type(r[key]) is int) for key in tokens},
             'unknown_token_tasks':{key:sum(type(r[key]) is not int for r in rows) for key in tokens},
             'provider_failures':sum(r['status']=='provider_failed' for r in rows),
             'provider_timeouts':sum(r['status']=='provider_timed_out' for r in rows),
             'measured_tasks':sum(type(r['input_tokens']) is int for r in rows),
             'known_proxy_subtotal':{'label':'proxy measured subtotal; excludes unknown tasks',
                           'lower_usd':str(sum(Decimal(r['proxy_usd_lower']) for r in rows if r['proxy_usd_lower']!='')),
                           'upper_usd':str(sum(Decimal(r['proxy_usd_upper']) for r in rows if r['proxy_usd_upper']!='')),
                           'measured_tasks':sum(r['proxy_usd_lower']!='' for r in rows)},
             'proxy_cost':{'label':'proxy','lower_usd':str(sum(Decimal(r['proxy_usd_lower']) for r in rows)),
                           'upper_usd':str(sum(Decimal(r['proxy_usd_upper']) for r in rows)),
                           'unknown_fields':['context_band','cache_write_tokens']} if all(r['proxy_usd_lower']!='' for r in rows) else None,
             'proxy_mapping':load_price_table(),
             'scope':'local official Harbor reproduction; incomplete campaigns have no accuracy score; smoke is not full acceptance'}
    output.mkdir(parents=True,exist_ok=False)
    with (output/'task-results.csv').open('w',newline='') as f:
        writer=csv.DictWriter(f,fieldnames=list(rows[0]));writer.writeheader();writer.writerows(rows)
    (output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    return summary


def plot(summaries, output):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    fig,(token_ax,proxy_ax)=plt.subplots(1,2,figsize=(12,4.5),sharey=True)
    colors=plt.rcParams['axes.prop_cycle'].by_key()['color']
    for index,summary in enumerate(summaries):
        accuracy=summary.get('accuracy_percent');denominator=summary['denominator']
        label=('TB2' if summary.get('benchmark')=='terminal_bench_2' else 'SWE Mini')+' '+summary['phase']
        label+=f" ({summary.get('successes',summary.get('resolved'))}/{denominator})"
        color=colors[index % len(colors)]
        if accuracy is None:
            label=('TB2' if summary.get('benchmark')=='terminal_bench_2' else 'SWE Mini')+' '+summary['phase']+f" ({summary.get('scored',summary.get('attempted'))}/{denominator} scored)"
            for ax in (token_ax,proxy_ax):ax.plot([],[],color=color,label=label+'; incomplete')
            continue
        counts=summary.get('tokens') or {}
        if all(type(counts.get(k)) is int for k in ['input_tokens','output_tokens']):
            total=counts['input_tokens']+counts['output_tokens']
            token_ax.scatter(total/denominator,accuracy,color=color,label=label)
        elif (known:=summary.get('known_token_subtotal')) and summary.get('measured_tasks'):
            token_ax.scatter((known['input_tokens']+known['output_tokens'])/denominator,accuracy,color=color,marker='>',label=label+'; lower bound')
        else:token_ax.plot([],[],color=color,label=label+'; tokens unknown')
        quote=summary.get('proxy_cost')
        if quote:
            lo=float(quote['lower_usd'])/denominator;hi=float(quote['upper_usd'])/denominator
            proxy_ax.plot([lo,hi],[accuracy]*2,color=color,marker='|',linewidth=3,label=label)
        elif (known:=summary.get('known_proxy_subtotal')) and known['measured_tasks']:
            lo=float(known['lower_usd'])/denominator
            proxy_ax.scatter(lo,accuracy,color=color,marker='>',label=label+'; lower bound')
            proxy_ax.annotate('Total upper bound unknown',xy=(lo,accuracy),xytext=(12,-18),textcoords='offset points',fontsize=8)
        else:proxy_ax.plot([],[],color=color,label=label+'; proxy unknown')
    token_ax.set(xlabel='Provider tokens per task (input + output)',ylabel='Official task accuracy (%)',title='Primary metric: tokens')
    token_ax.ticklabel_format(axis='x',style='sci',scilimits=(0,0))
    proxy_ax.set(xlabel='Proxy API cost per task (USD interval)',title='Proxy: context band/cache writes unknown')
    for ax in (token_ax,proxy_ax):
        ax.set_ylim(0,100);ax.grid(alpha=.2);ax.legend(loc='lower right',fontsize=8)
    fig.suptitle('MP-08 / MP-10: cost versus accuracy; cached/reasoning tokens included once')
    fig.tight_layout();fig.savefig(output,dpi=160);plt.close(fig)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--campaign',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--compare-summary',type=Path,action='append',default=[]);a=p.parse_args()
    if not a.output.is_absolute() or a.output.resolve().is_relative_to(Path(__file__).resolve().parents[5]):raise ValueError('MP-11: external evidence output required')
    summary=report(a.campaign,a.output)
    plot([summary,*[json.loads(s.read_text()) for s in a.compare_summary]],a.output/'cost-vs-accuracy.png')
    print('MP-08 / MP-10: official terminal report written; complete='+str(summary['complete']))


if __name__=='__main__':main()
