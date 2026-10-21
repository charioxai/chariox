# MP-08/MP-10/MP-11: only finite numeric timing records, never runtime payloads.
import json,sys,math
from pathlib import Path
root=Path(sys.argv[1]);out=Path(sys.argv[2]);assert str(root).startswith('/root/.chariox/dev/vidlat/r6/drills/')
rows=[]
for p in root.rglob('display-timing.jsonl'):
 for line in p.read_text().splitlines():
  try:r=json.loads(line)
  except json.JSONDecodeError:continue
  if isinstance(r.get('stage'),str) and len(r['stage'])<90 and all(isinstance(v,(int,float)) and math.isfinite(v) for k,v in r.items() if k!='stage'):rows.append(r)
for p in (root/'logs').glob('kernel.log'):
 for line in p.read_text().splitlines():
  if not line.startswith('MD-DISPLAY-TIMING '):continue
  try:r=json.loads(line[len('MD-DISPLAY-TIMING '):])
  except json.JSONDecodeError:continue
  if isinstance(r.get('stage'),str) and r['stage'].replace('_','').isalnum() and all(isinstance(v,(int,float)) and math.isfinite(v) for k,v in r.items() if k!='stage'):rows.append(r)
out.write_text(''.join(json.dumps(r)+'\n'for r in rows));print({'MP':['MP-08','MP-10','MP-11'],'numeric_rows':len(rows)})
