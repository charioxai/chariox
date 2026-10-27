# MP-08 / MP-10 / MP-11: only non-secret drill history; never account/Vault state.
import sqlite3, json, sys, hashlib
from pathlib import Path
root, session, agent = sys.argv[1:4]
db = Path(root) / 'home/history/operational.db'
assert db.is_file() and (Path(root)/'context-switch-owned.json').is_file()
c = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
events = c.execute('SELECT sequence,kind,prompt_id,provider,model,length(CAST(content AS BLOB)),event_json FROM history_events WHERE session_id=? AND agent_id=? ORDER BY sequence', (session, agent)).fetchall()
out = {'eventCount': len(events), 'contentBytes': sum(e[5] or 0 for e in events), 'lastSequence': max((e[0] for e in events), default=0), 'prompts': [], 'handoffs': []}
for seq, kind, prompt, provider, model, size, raw in events:
    e = json.loads(raw)
    if kind == 'user_prompt':
        out['prompts'].append({'sequence': seq, 'id': prompt, 'provider': provider, 'model': model, 'bytes': size})
    meta = e.get('metadata', {})
    allowed = ['target_provider_run_id','source_provider','source_model','target_provider','target_model','derived','brief_bytes','handoff_bytes','history_bytes','omitted_turns','elapsed_ms','calls']
    if any(k in meta for k in ['handoff_bytes','brief_bytes','omitted_turns']):
        out['handoffs'].append({'sequence': seq, **{k: meta[k] for k in allowed if k in meta}})
try:
    row = c.execute('SELECT brief,covered_through_sequence FROM history_agent_handoff_briefs WHERE session_id=? AND agent_id=?', (session, agent)).fetchone()
    if row:
        brief, watermark = row
        out['brief'] = {'coveredThroughSequence': watermark,'bytes':len(brief.encode()),'sha256':hashlib.sha256(brief.encode()).hexdigest(),'sections':[line for line in brief.splitlines() if line.startswith('## ')], 'text': brief}
except sqlite3.OperationalError:
    out['briefTableAbsent'] = True
print(json.dumps(out))
