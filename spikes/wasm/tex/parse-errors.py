#!/usr/bin/env python3
"""MP-08/MP-10: conservative structured TeX errors for agent actions."""
import json,re,sys

def parse(text):
 errors=[];pending=None
 for line in text.splitlines():
  line=re.sub(r'^\[[^\]]+\]\s*','',line)
  match=re.match(r'^(?:(?:tectonic )?(?:error|warning): )?(.+\.(?:tex|sty|cls|bib)):(\d+):\s*(.*)',line)
  if match:
   errors.append({'file':match[1],'line':int(match[2]),'message':match[3]});pending=None
  elif line.startswith('! '):pending=line[2:].strip()
  elif pending and (m:=re.match(r'l\.(\d+)\s*(.*)',line)):
   # A plain TeX log does not always identify the active file reliably.
   if not any(e['line']==int(m[1]) and e['message'].rstrip('.')==pending.rstrip('.') for e in errors):
    errors.append({'file':None,'line':int(m[1]),'message':pending})
   pending=None
 return errors
if __name__=='__main__':print(json.dumps(parse(sys.stdin.read()),indent=2))
