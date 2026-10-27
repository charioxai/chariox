# MP-08 / MP-10 / MP-11: owned worker source-agent history, exact original marker only.
import sqlite3,json,sys,subprocess
from pathlib import Path
root=sys.argv[1];db=Path(root)/'home/history/operational.db'
assert db.is_file() and (Path(root)/'context-switch-owned.json').is_file()
c=sqlite3.connect(f'file:{db}?mode=ro',uri=True)
keys=c.execute("SELECT DISTINCT session_id,agent_id FROM history_events WHERE kind='user_prompt' AND instr(content,?)>0",('CTXVAL_SOURCE_FACTS',)).fetchall()
rows=[]
for session,agent in keys:
 result=subprocess.check_output([sys.executable,str(Path(__file__).with_name('history-observe.py')),root,session,agent],text=True)
 rows.append({'sessionId':session,'agentId':agent,'history':json.loads(result)})
print(json.dumps({'workerSourceHistories':rows}))
