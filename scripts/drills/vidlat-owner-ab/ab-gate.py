#!/usr/bin/env python3
# MP-08/MP-10/MP-11: add/remove only the owned stack's Selkies route.
# The existing valenv private peer gate and shared Caddy root remain unchanged.
import fcntl,ipaddress,json,re,subprocess,sys
from pathlib import Path

action,name=sys.argv[1:3]
if action not in ('attach','detach') or not re.fullmatch(r'pr[0-9]+-vidlat[a-z0-9-]*',name):
    raise SystemExit('MP-11: own vidlat stack required')
base=Path('/w/valenv');lane=base/'prs'/name
with (base/'prs/.lock').open('a') as lock:
    fcntl.flock(lock,fcntl.LOCK_EX)
    metadata=json.loads((lane/'metadata.json').read_text())
    if metadata['project']!='chariox-valenv-'+name:raise SystemExit('MP-11: stack identity mismatch')
    config=lane/'Caddyfile';backup=lane/'ab-caddy-original'
    if action=='attach':
        if not metadata.get('desired_running'):raise SystemExit('MP-11: start the own stack first')
        address=str(ipaddress.IPv4Address(sys.argv[3]));port=int(sys.argv[4])
        if not 1024<=port<=65535:raise SystemExit('MP-11: unsafe upstream port')
        if backup.exists():raise SystemExit('MP-11: detach existing own A/B route first')
        original=config.read_text();backup.write_text(original)
        host=metadata['hostname'].replace(name+'.',name+'-selkies.',1)
        route=f'\nhttp://{host} {{\n route {{\n import private\n redir https://{{host}}{{uri}} 308\n }}\n}}\n{host} {{\n route {{\n import private\n reverse_proxy {address}:{port} {{\n stream_close_delay 4h\n }}\n }}\n}}\n'
        config.write_text(original+route)
    else:
        if not backup.exists():raise SystemExit('MP-11: no owned A/B route to detach')
        if metadata.get('desired_running'):config.write_text(backup.read_text())
        else:config.unlink(missing_ok=True)
    try:
        for cmd in ['validate','reload']:
            result=subprocess.run([str(base/'bin/caddy'),cmd,'--config',str(base/'scripts/Caddyfile'),'--adapter','caddyfile'],capture_output=True)
            if result.returncode:raise RuntimeError('MP-11: Caddy '+cmd+' failed; inspect private operator logs')
    except Exception:
        if action=='attach':config.write_text(backup.read_text());backup.unlink()
        raise
    if action=='detach':backup.unlink()
    print('MP-08/MP-10/MP-11: owned gated Selkies route '+action+' complete')
