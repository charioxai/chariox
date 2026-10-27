#!/usr/bin/env python3
# MP-08/MP-10/MP-11: validate public inputs, own PID identities and exact firewall rule.
import ipaddress,json,os,re,shutil,signal,subprocess,sys,time
from pathlib import Path
from urllib.parse import urlsplit

root=Path(os.environ['AB_STATE'])
if not root.is_absolute() or '..' in root.parts or not re.fullmatch(r'/root/\.chariox/dev/vidlat/owner-ab-[a-zA-Z0-9_-]+',str(root)):
    raise SystemExit('MP-11: use /root/.chariox/dev/vidlat/owner-ab-<unique-run>')
if root.is_symlink():raise SystemExit('MP-11: state cannot be a symlink')
def identity(pid):
    if type(pid) is not int or pid<=1:raise RuntimeError('MP-11: unsafe PID')
    try:
        fields=(Path('/proc')/str(pid)/'stat').read_text().rsplit(')',1)[1].split()
        return {'pid':pid,'ppid':int(fields[1]),'start':fields[19],'state':fields[0]}
    except (FileNotFoundError,ProcessLookupError):return None
def alive(row):
    current=identity(row['pid'])
    return current and current['start']==row['start'] and current['state']!='Z'
def tree(seed):
    rows={}
    for path in Path('/proc').iterdir():
        if path.name.isdigit() and int(path.name)>1:
            try:
                row=identity(int(path.name))
                if row:rows[row['pid']]=row
            except (OSError,ValueError):pass
    owned={seed['pid']:seed} if alive(seed) else {}
    for _ in range(40):
        extra={pid:r for pid,r in rows.items() if r['ppid'] in owned and alive(owned[r['ppid']])}
        count=len(owned);owned.update(extra)
        if len(owned)==count:break
    return owned
def public_origin(key,scheme):
    u=urlsplit(os.environ[key])
    if u.scheme!=scheme or not u.hostname or u.username or u.password or u.query or u.fragment or u.path not in ('','/'):
        raise SystemExit('MP-11: credential-free TLS origin required: '+key)
mode=sys.argv[1]
if mode=='preflight':
    for key,scheme in [('AB_ORIGIN','https'),('AB_SELKIES_ORIGIN','https')]:public_origin(key,scheme)
    if os.environ.get('AB_RELAY_URL'):public_origin('AB_RELAY_URL','wss')
    if os.environ.get('AB_API_ORIGIN'):public_origin('AB_API_ORIGIN','https')
    ipaddress.IPv4Address(os.environ['AB_GATEWAY_IP'])
    free=shutil.disk_usage('/').free
    available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024
    if free<10*1024**3 or available<12*1024**3:raise SystemExit('MP-10: resource floor not met')
    print('MP-08/MP-10/MP-11: resource/origin preflight passed')
elif mode=='record':
    row=identity(int(sys.argv[2]))
    if not row:raise SystemExit('MP-11: supervisor missing')
    (root/'owned.json').write_text(json.dumps(row));os.chmod(root/'owned.json',0o600)
elif mode=='firewall':
    port=int(sys.argv[2]);gateway=str(ipaddress.IPv4Address(os.environ['AB_GATEWAY_IP']))
    if not 1024<=port<=65535:raise SystemExit('MP-11: invalid own listener port')
    args=['INPUT','!','-i','lo','-p','tcp','--dport',str(port),'!','-s',gateway,'-m','comment','--comment',root.name,'-j','DROP']
    # Save before mutation so an interrupted install can always settle its own rule.
    (root/'firewall.json').write_text(json.dumps(args))
    subprocess.run(['iptables','-I',*args],check=True)
elif mode=='stop':
    if root.exists() and (not (root/'.vidlat-owned').exists() or (root/'.vidlat-owned').read_text().strip()!='vidlat-owner-ab'):
        raise SystemExit('MP-11: state ownership marker missing; no cleanup performed')
    evidence=Path(os.environ.get('AB_EVIDENCE','/root/.codex/evidence/browser-resume-20260930/vidlat/r9'))
    evidence.mkdir(parents=True,exist_ok=True)
    signals=[];remaining=[]
    if (root/'owned.json').exists():
        seed=json.loads((root/'owned.json').read_text());owned=tree(seed)
        if (root/'child-identities.json').exists():
            for child in json.loads((root/'child-identities.json').read_text()):owned.update(tree(child))
        # Retain every verified descendant before the supervisor can exit/reparent it.
        for sig in [signal.SIGTERM,signal.SIGKILL]:
            owned.update(tree(seed))
            if alive(seed):
                os.kill(seed['pid'],sig);signals.append({'pid':seed['pid'],'start':seed['start'],'signal':sig.name})
            for _ in range(100):
                if not any(alive(r) for r in owned.values()):break
                time.sleep(.1)
            # Catch orphaned children only from the verified pre-stop descendant set.
            for row in owned.values():
                if alive(row):
                    os.kill(row['pid'],sig);signals.append({'pid':row['pid'],'start':row['start'],'signal':sig.name})
            for _ in range(50):
                if not any(alive(r) for r in owned.values()):break
                time.sleep(.1)
        remaining=[r for r in owned.values() if alive(r)]
    if (root/'firewall.json').exists():
        args=json.loads((root/'firewall.json').read_text())
        if args[0]!='INPUT' or root.name not in args:raise RuntimeError('MP-11: firewall ownership mismatch')
        if subprocess.run(['iptables','-C',*args],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0:
            subprocess.run(['iptables','-D',*args],check=True)
    report={'items':['MP-08','MP-10','MP-11'],'state':str(root),'signals':signals,'remaining':remaining,'cleanup':'FAIL' if remaining else 'PASS'}
    (evidence/(root.name+'-cleanup.json')).write_text(json.dumps(report,indent=2)+'\n')
    if remaining:raise SystemExit('MP-10: owned process remains; state retained')
    # Explicit disposable root: only generated runtime identities; no provider profile/key inputs.
    if (root/'workspace.json').exists():
        workspace=Path(json.loads((root/'workspace.json').read_text())['path'])
        expected=Path('/root/work')/('agent-vidlat-owner-ab-'+root.name)
        if workspace!=expected or workspace.is_symlink():raise RuntimeError('MP-11: workspace ownership mismatch')
        if workspace.exists():
            marker=workspace/'.vidlat-owner-ab'
            if not marker.exists() or marker.read_text().strip()!='vidlat-owner-ab':raise RuntimeError('MP-11: workspace marker missing; state retained')
            shutil.rmtree(workspace)
    if root.exists():shutil.rmtree(root)
    print('MP-08/MP-10/MP-11: exact owned processes stopped; disposable state and rule removed')
else:raise SystemExit('MP-11: unsupported operation')
