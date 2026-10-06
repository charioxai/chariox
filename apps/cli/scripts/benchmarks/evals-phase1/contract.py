"""MP-08 / MP-10 / MP-11: fail-closed benchmark measurements, no provider execution."""
import re


def validate_lock(lock):
    for key, count in [('terminal_bench_2', 89), ('swe_verified_mini', 50)]:
        entry = lock[key]
        ids = entry['task_ids']
        if entry['task_count'] != count or len(ids) != count or len(set(ids)) != count:
            raise ValueError(f'{key}: full pinned denominator required')
        if not re.fullmatch(r'[0-9a-f]{40}', entry['revision']):
            raise ValueError(f'{key}: exact revision required')


def safe_pid(pid):
    if type(pid) is not int or pid <= 1:
        raise ValueError('MP-11: refusing unsafe signal PID')
    return pid


def completed_answer(snapshot, session_id, agent_id, prompt_id):
    session = snapshot.get('session', {})
    if session.get('id') != session_id:
        return None
    agent = next((a for a in session.get('agents', []) if a['id'] == agent_id), None)
    if not agent or agent.get('state') not in {'Idle', 'Done'}:
        return None
    transcript = snapshot.get('transcript', {})
    if transcript.get('visibleAgentId') != agent_id:
        return None
    entries = [e for e in transcript.get('entries', [])
               if e.get('role') == 'assistant' and e.get('promptId') == prompt_id
               and not e.get('hidden')]
    return '\n'.join(e['text'] for e in entries) if entries else None


def admit_token_usage(report, session_id):
    if report.get('session_id') != session_id or report.get('complete') is not True:
        raise ValueError('MP-08 / MP-10: missing or incomplete kernel accounting')
    required = ['input_tokens', 'cached_input_tokens', 'output_tokens']
    if any(type(report.get(k)) is not int or report[k] < 0 for k in required):
        raise ValueError('MP-08 / MP-10: unavailable accounting cannot be reported as zero')
    if report['cached_input_tokens'] > report['input_tokens']:
        raise ValueError('MP-08 / MP-10: inconsistent cache count')
    return report


def admit_usage(report, session_id):
    admit_token_usage(report, session_id)
    cost = report.get('api_equivalent_nanodollars')
    if type(cost) is not int or cost < 0:
        raise ValueError('MP-08 / MP-10: unavailable price cannot be reported as zero')
    return report


def quota_exhausted(meters, now_ms):
    # Empty purchased credits do not prove a subscription plan is exhausted.
    return any(m.get('kind') == 'rolling_limit' and m.get('state') == 'exhausted'
               and type(m.get('observed_at_ms')) is int
               and 0 <= now_ms - m['observed_at_ms'] <= 300_000
               and type(m.get('resets_at_ms')) is int and m['resets_at_ms'] > now_ms
               for m in meters)


def admit_placement(request):
    placement = request.get('placement', 'local')
    if placement == 'local':
        from pathlib import Path
        path = request.get('profile_path')
        if not path or not Path(path).is_absolute() or '/.codex-agents' in path:
            raise ValueError('MP-08 / MP-10: approved product-linked profile required')
    elif placement == 'leased':
        if request.get('profile_path'):
            raise ValueError('MP-08 / MP-10: leased access uses the home grant, not a builder profile')
        for field in ['home_kernel_url', 'home_session_ref', 'home_agent_id', 'worker_kernel_id']:
            if not request.get(field):
                raise ValueError(f'MP-08 / MP-10: coordinator lease binding requires {field}')
    else:
        raise ValueError('MP-08 / MP-10: unsupported placement')
    return placement


def leased_target(snapshot, request):
    if snapshot.get('session', {}).get('id') != request['home_session_ref']:
        raise ValueError('MP-08 / MP-10: canonical home session binding mismatch')
    agent = next((a for a in snapshot.get('session', {}).get('agents', [])
                  if a.get('id') == request['home_agent_id']), None)
    remote = (agent or {}).get('remoteExecution') or {}
    if not agent or remote.get('workerKernelId') != request['worker_kernel_id']:
        raise ValueError('MP-08 / MP-10: foreign or missing home-worker lease binding')
    if agent.get('provider') != request['provider'] or agent.get('model') != request['model']:
        raise ValueError('MP-08 / MP-10: leased provider/model does not match the task')
    return agent


def task_usage(report, session_id, agent_id, prompt_id, intended_bindings=()):
    """MP-08 / MP-10: only run records bound to this submitted task prompt."""
    if report.get('session_id') != session_id:
        raise ValueError('MP-08 / MP-10: foreign session accounting')
    bindings={(agent_id,prompt_id)}
    for binding in intended_bindings:
        if not isinstance(binding,(list,tuple)) or len(binding)!=2 or not all(isinstance(v,str) and v for v in binding):
            raise ValueError('MP-08 / MP-10: explicit descendant agent/prompt binding required')
        bindings.add(tuple(binding))
    turns=[t for t in report.get('turns',[]) if (t.get('agent_id'),t.get('prompt_id')) in bindings]
    if {(t.get('agent_id'),t.get('prompt_id')) for t in turns} != bindings:return None
    if not turns or not all(t.get('completed') is True and t.get('usage') is not None for t in turns):
        return None
    run_ids=[t['provider_run_id'] for t in turns]
    if len(set(run_ids))!=len(run_ids):
        raise ValueError('MP-08 / MP-10: duplicate task run accounting')
    usage={}
    for key in ['input_tokens','cached_input_tokens','output_tokens','reasoning_tokens','cache_write_tokens','cache_write_5m_tokens','cache_write_1h_tokens']:
        source_key=key.removesuffix('_tokens') if key.startswith('cache_write') else key
        values=[t['usage'].get(source_key) for t in turns]
        usage[key]=sum(values) if all(type(n) is int and n>=0 for n in values) else None
    if any(usage[k] is None for k in ['input_tokens','cached_input_tokens','output_tokens']):return None
    costs=[t.get('api_equivalent_nanodollars') for t in turns]
    return {'session_id':session_id,'complete':True,'provider_run_ids':run_ids,**usage,
            'api_equivalent_nanodollars':sum(int(n) for n in costs) if all(n is not None for n in costs) else None}


def session_usage_visible(report, visible):
    """MP-08 / MP-10: the ordinary TUI reports session totals, independently of task scope."""
    total=report.get('total') or {}
    usage=total.get('usage') or {}
    if not usage:return False
    counters='; '.join(f'{label} {usage[key] if usage.get(key) is not None else "unavailable"}'
                      for label,key in [('input','input_tokens'),('cached','cached_input_tokens'),('output','output_tokens'),('reasoning','reasoning_tokens')])
    return f'Session: {total.get("turns")} turns; {counters};' in " ".join(visible.split())
