# MP-08 / MP-10 / MP-11: owned native hook metadata and terminal classifications, no prompt/auth payloads.
from pathlib import Path
import json,sys
r=Path(sys.argv[1]);assert r.is_absolute() and r.name.startswith('worker-state-')
hooks=[];bindings=[];screens=[]
for p in r.rglob('events.jsonl'):
 es=[]
 for line in p.read_text(errors='replace').splitlines():
  try:
   e=json.loads(line);es.append({k:e[k] for k in ['hook_event_name','native_prompt_id','hook_context_request_id','session_id','hook_context_deadline_ms'] if k in e})
  except ValueError:pass
 hooks.append({'path':str(p.relative_to(r)),'events':es[-15:]})
for p in r.rglob('*.context-delivery'):
 try:
  e=json.loads(p.read_text());bindings.append({k:e[k] for k in ['request_id','run_id','prompt_id','active_prompt_id','managed'] if k in e})
 except ValueError:pass
for p in r.rglob('permission-recent.txt'):
 t=p.read_text(errors='replace');screens.append({'bytes':len(t.encode()),'classes':{k:k in t for k in ['Interrupted','interrupted','What should Claude do instead','UserPromptSubmit','operation blocked','Chariox could not prepare','Thinking','Swooping','esc to interrupt','API Error','Credit balance','Press Enter','StopFailure']}})
print(json.dumps({'hooks':hooks,'bindings':bindings,'screens':screens}))
