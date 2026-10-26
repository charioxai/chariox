# MP-08 / MP-10 / MP-11: allowlisted diagnostic fields only, never raw logs.
import json, sys
from pathlib import Path
root = Path(sys.argv[1])
assert (root/'context-switch-owned.json').is_file()
rows = []
fields = ['provider_run_id','session_id','agent_id','target_provider_run_id','source_provider','source_model','target_provider','target_model','utility_calls','brief_bytes','context_bytes','max_bytes','elapsed_ms','brief_model','context_tokens','source_window','target_window','source_context_window','target_context_window']
for path in (root/'home/logs').glob('*-daemon-*.ndjson'):
    for line in path.open():
        try: e = json.loads(line)
        except ValueError: continue
        if e.get('component') != 'daemon.provider_context_handoff': continue
        f = e.get('fields') or e
        details = f.get('details') if isinstance(f.get('details'), dict) else {}
        observed = {**details, **f}
        error = str(f.get('error','')).lower()
        category = 'rejected-model' if any(x in error for x in ['unsupported model','model_not_found','model not found','not supported when using codex']) else 'auth-rejection' if any(x in error for x in ['401','unauthorized']) else 'invalid-brief' if 'not a handoff brief' in error else 'quota' if any(x in error for x in ['usage limit','quota','rate limit']) else 'timeout' if any(x in error for x in ['deadline','timeout','timed out']) else 'other-error' if error else None
        rows.append({'atMs': e.get('timestamp_ms'),'level':e.get('level'),'message':e.get('message'), 'fields':{k:observed[k] for k in fields if k in observed},'errorClass':category})
print(json.dumps({'rows':rows}))
