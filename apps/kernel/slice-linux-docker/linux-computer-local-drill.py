#!/usr/bin/env python3
"""MP-08 / MP-11: copy exact public helpers for a non-root disposable local drill."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

parser=argparse.ArgumentParser()
parser.add_argument('drill',choices=['linux-owned-desktop','native-computer','native-accessibility'])
parser.add_argument('--evidence',required=True)
args=parser.parse_args()
source=Path(__file__).resolve().parent
files=['linux-owned-desktop.mjs','linux-owned-process.mjs','native-computer.mjs','native-computer.py','slice-keyboard.py','slice-text-finder.py','x11-text-keyboard.py']
files += [name for name in ['native-accessibility.mjs','native-accessibility.py'] if (source/'docker'/name).exists()]
head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=source,text=True).strip()
evidence=Path(args.evidence);evidence.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='culinux-b-source-') as temporary:
    root=Path(temporary);(root/'docker').mkdir();os.chmod(root,0o755)
    hashes={}
    for name in files:
        shutil.copy(source/'docker'/name,root/'docker'/name)
        hashes[name]=hashlib.sha256((root/'docker'/name).read_bytes()).hexdigest()
    drill=args.drill+'-drill.mjs';shutil.copy(source/drill,root/drill)
    command=['runuser','-u','nobody','--','env','CULINUX_SOURCE='+head+'+recorded-working-tree','node',str(root/drill)] if os.getuid()==0 else ['node',str(root/drill)]
    result=subprocess.run(command,capture_output=True,text=True,timeout=120)
    (evidence/(args.drill+'.log')).write_text(result.stdout+result.stderr)
    (evidence/(args.drill+'.json')).write_text(json.dumps({'items':['MP-08','MP-11'],'head':head,'helper_sha256':hashes,'command':command,'exit':result.returncode,'cleanup':'owned desktop and scratch removed in drill finally; copied public source removed'},indent=2)+'\n')
    print(result.stdout,end='');print(result.stderr,end='');print('MP-08 MP-11 exit='+str(result.returncode))
    status=result.returncode
raise SystemExit(status)
