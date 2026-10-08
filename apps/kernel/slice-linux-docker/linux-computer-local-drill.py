#!/usr/bin/env python3
"""MP-08 / MP-11: copy exact public helpers for a non-root disposable local drill."""
import argparse
import hashlib
import json
import os
import signal
import time
from pathlib import Path
import shutil
import subprocess
import tempfile

parser=argparse.ArgumentParser()
parser.add_argument('drill',choices=['linux-owned-desktop','native-computer','native-accessibility'])
parser.add_argument('--evidence',required=True)
parser.add_argument('--desktop-capture',action='store_true',help='MP-08/MP-11: supplementary protected desktop source check')
parser.add_argument('--native-prefix',help='MP-08: optional lane-owned public native dependencies')
args=parser.parse_args()
source=Path(__file__).resolve().parent
files=['native-x11.py','browser-desktop-protection.py','browser-protection-regions.mjs','kernel-browser-refusal.mjs','native-keyboard-channel.mjs','browser-controller-snapshot.mjs','linux-owned-desktop.mjs','linux-owned-process.mjs','linux-desktop-session.py','native-computer.mjs','native-computer.py','native-clipboard.py','slice-keyboard.py','slice-text-finder.py','x11-text-keyboard.py']
files += [name for name in ['native-accessibility.mjs','native-accessibility.py','room-native-protection.py'] if (source/'docker'/name).exists()]
if args.desktop_capture:
    files=sorted(path.name for path in (source/'docker').iterdir() if path.suffix in ('.mjs','.py'))
head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=source,text=True).strip()
evidence=Path(args.evidence);evidence.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='cn-',dir='/var/tmp') as temporary:
    root=Path(temporary);(root/'docker').mkdir();os.chmod(root,0o755)
    hashes={}
    for name in files:
        shutil.copy(source/'docker'/name,root/'docker'/name)
        hashes[name]=hashlib.sha256((root/'docker'/name).read_bytes()).hexdigest()
    drill=args.drill+'-drill.mjs';shutil.copy(source/drill,root/drill)
    fixture_hash=None
    if args.desktop_capture:
        shutil.copy(source/'native-accessibility-fixture.py',root/'native-accessibility-fixture.py')
        fixture_hash=hashlib.sha256((root/'native-accessibility-fixture.py').read_bytes()).hexdigest()
    if args.drill=='native-accessibility':
        shutil.copy(source/'native-accessibility-fixture.py',root/'native-accessibility-fixture.py')
        fixture_hash=hashlib.sha256((root/'native-accessibility-fixture.py').read_bytes()).hexdigest()
    command=['/usr/bin/node',str(root/drill)]
    manifest={'items':['MP-08','MP-11'],'head':head,'helper_sha256':hashes,'drill_sha256':hashlib.sha256((root/drill).read_bytes()).hexdigest(),'command':command,'fixture_sha256':fixture_hash,'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'exit':None}
    receipt=evidence/(args.drill+'.json');receipt.write_text(json.dumps(manifest,indent=2)+'\n')
    tracked=[str(source/'docker'/name) for name in files]+[str(source/drill),str(Path(__file__).resolve())]
    if fixture_hash:tracked.append(str(source/'native-accessibility-fixture.py'))
    dirty=subprocess.check_output(['git','status','--porcelain','--',*tracked],cwd=source,text=True).strip()
    identity=head+('+recorded-working-tree' if dirty else '')
    manifest['source_identity']=identity
    environment={'PATH' :'/usr/bin:/bin','HOME':'/nonexistent','LANG':'C.UTF-8','CULINUX_SOURCE':identity}
    state=root/'s';state.mkdir(mode=0o700)
    captures=root/'captures';captures.mkdir(mode=0o700)
    if os.getuid()==0:os.chown(state,65534,65534)
    if os.getuid()==0:os.chown(captures,65534,65534)
    environment['TMPDIR']=str(state)
    environment['CULINUX_CAPTURE_ROOT']=str(captures)
    if args.desktop_capture:environment['CULINUX_DESKTOP_CAPTURE']='1'
    if args.native_prefix:
        prefix=Path(args.native_prefix).resolve(strict=True)
        environment['CULINUX_NATIVE_PREFIX']=str(prefix)
    options={'user':65534,'group':65534,'extra_groups':[]} if os.getuid()==0 else {}
    log=evidence/(args.drill+'.log');output=log.open('w')
    process=subprocess.Popen(command,stdout=output,stderr=subprocess.STDOUT,text=True,env=environment,**options)
    if process.pid<=1:raise RuntimeError('MP-11: invalid owned drill PID')
    stat=Path('/proc/'+str(process.pid)+'/stat').read_text();identity=stat[stat.rfind(')')+2:].split()[19]
    def stop_owned(number):
        if not isinstance(process.pid,int) or process.pid<=1:raise RuntimeError('MP-11: invalid drill signal target')
        try:
            value=Path('/proc/'+str(process.pid)+'/stat').read_text()
            if value[value.rfind(')')+2:].split()[19]!=identity:raise RuntimeError('MP-11: drill identity changed')
            os.kill(process.pid,number)
        except (FileNotFoundError,ProcessLookupError):pass
    try:
        process.wait(timeout=120);status=process.returncode
    except subprocess.TimeoutExpired:
        stop_owned(signal.SIGTERM)
        try:process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            stop_owned(signal.SIGKILL);process.wait(timeout=5)
        status=124
    output.close();stdout=log.read_text();stderr=''
    for capture in captures.glob('*.png'):shutil.copy(capture,evidence/capture.name)
    manifest.update(exit=status,cleanup='copied public source removed by runner; fixture cleanup is asserted on exit=0 and reported in log',cleanup_proven=status==0)
    manifest['resources']={'disk_free_bytes':shutil.disk_usage('/').free,'mem_available_kib':next(int(line.split()[1]) for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))}
    receipt.write_text(json.dumps(manifest,indent=2)+'\n')
    print(stdout,end='');print(stderr,end='');print('MP-08 MP-11 exit='+str(status))
raise SystemExit(status)
