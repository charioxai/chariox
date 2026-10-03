#!/usr/bin/env python3
"""Read-only admission for support overlays; never stop an existing slice."""
import argparse
import json
import os
from pathlib import PurePosixPath
import re
import subprocess
import sys


# Runs as root only to read process identity and the slice user's private record.
# It does not import the old or new lifecycle implementation or write receipts.
CENSUS = r'''
import hashlib,json,os,stat,sys,time
from pathlib import Path
profile,uid=sys.argv[1],int(sys.argv[2])
deadline=time.monotonic()+8
boot=Path('/proc/sys/kernel/random/boot_id').read_text().strip()
namespace=os.readlink('/proc/self/ns/pid')
def identity(pid):
    fields=Path(f'/proc/{pid}/stat').read_text().rpartition(')')[2].split()
    if fields[0]=='Z': return None
    return {'pid':pid,'start':fields[19],'group':int(fields[2]),'session':int(fields[3]),
            'uid':Path(f'/proc/{pid}').stat().st_uid,'boot':boot,'namespace':namespace}
def profile_arguments(command,profile):
    args=command.split(b'\0');value=profile.encode()
    return b'--user-data-dir='+value in args or any(arg==b'--user-data-dir' and index+1<len(args) and args[index+1]==value for index,arg in enumerate(args))
def read_record(pointer,uid):
    def version(info):
        return (info.st_dev,info.st_ino,info.st_size,info.st_mtime_ns,info.st_ctime_ns,info.st_uid,info.st_mode)
    fd=os.open(pointer,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    with os.fdopen(fd,'rb') as stream:
        before=os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_uid!=uid or stat.S_IMODE(before.st_mode)!=0o600 or before.st_size>4096: raise ValueError()
        payload=stream.read(4097)
        after=os.fstat(stream.fileno())
        named=pointer.lstat()
        if len(payload)>4096 or len(payload)!=before.st_size or version(before)!=version(after) or version(before)!=version(named): raise ValueError()
        return json.loads(payload)
parents={};matches=[]
paths=list(Path('/proc').glob('[0-9]*/cmdline'))
if len(paths)>4096: raise RuntimeError('process census exceeds bound')
for path in paths:
    if time.monotonic()>deadline: raise RuntimeError('process census timed out')
    try:
        pid=int(path.parent.name)
        fields=Path(f'/proc/{pid}/stat').read_text().rpartition(')')[2].split()
        parents[pid]=int(fields[1])
        if fields[0]=='Z' or pid==os.getpid(): continue
        with path.open('rb') as stream: command=stream.read(16385)
        if len(command)>16384: raise RuntimeError('process command exceeds bound')
        if not profile_arguments(command,profile): continue
        matches.append(identity(pid))
    except (FileNotFoundError,ProcessLookupError): pass
matches=[item for item in matches if item]
disposition='clear' if not matches else 'unowned'
if matches:
    try:
        root=Path(os.environ.get('CHARIOX_BROWSER_LIFECYCLE_ROOT',f'/tmp/chariox-browser-lifecycle-{uid}'))
        info=root.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid!=uid or stat.S_IMODE(info.st_mode)!=0o700 or root.resolve()!=root: raise ValueError()
        pointer=root/(hashlib.sha256(profile.encode()).hexdigest()+'.json')
        record=read_record(pointer,uid)
        owner=record['supervisor']
        if record['version']!=1 or record['profile']!=profile or owner['uid']!=uid or identity(owner['pid'])!=owner: raise ValueError()
        def descendant(pid):
            seen=set()
            while pid not in seen and pid in parents:
                if pid==owner['pid']: return True
                seen.add(pid);pid=parents[pid]
            return pid==owner['pid']
        if all(item['uid']==uid and descendant(item['pid']) and identity(item['pid'])==item for item in matches):
            disposition='owned'
    except (OSError,ValueError,KeyError,TypeError,json.JSONDecodeError): pass
print(json.dumps({'disposition':disposition,'profileProcessCount':len(matches)}))
'''


def generation(info):
    state = info.get('State', {})
    return {'Id': info.get('Id'), 'Image': info.get('Image'), 'Created': info.get('Created'),
            'labels': info.get('Config', {}).get('Labels'), 'pidMode': info.get('HostConfig', {}).get('PidMode'),
            'state': {key: state.get(key) for key in ('Running', 'Paused', 'Restarting', 'Status', 'Pid', 'StartedAt', 'FinishedAt')}}


def check(options, run):
    for name in ('container', 'slice_id', 'owner_kernel_id', 'owner_machine_id'):
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}', options[name]):
            raise RuntimeError('exact slice and container ownership identities required')
    if not isinstance(options['uid'], int) or options['uid'] <= 0:
        raise RuntimeError('non-root slice browser UID required')
    engine = run(['info', '--format', '{{.ID}}']).strip()
    if not engine or len(engine) > 128:
        raise RuntimeError('Docker engine identity unavailable')
    first = json.loads(run(['inspect', options['container']]))
    if not isinstance(first, list) or len(first) != 1:
        raise RuntimeError('exact slice container inspection required')
    first = first[0]
    environment = dict(item.split('=', 1) for item in first.get('Config', {}).get('Env', []) if '=' in item)
    profile = options.get('profile') or environment.get('CHARIOX_SLICE_CHROME_PROFILE') or environment.get('HOME', '/home/slice') + '/.chariox/browser/chromium'
    if not profile.startswith('/') or str(PurePosixPath(profile)) != profile or '..' in PurePosixPath(profile).parts or profile == '/':
        raise RuntimeError('canonical browser profile path required')
    container_id = first.get('Id', '')
    if not re.fullmatch(r'[0-9a-f]{64}', container_id):
        raise RuntimeError('full Docker container identity required')
    labels = first.get('Config', {}).get('Labels') or {}
    for label, name in [('io.chariox.slice.id', 'slice_id'), ('io.chariox.slice.owner-kernel-id', 'owner_kernel_id'),
                        ('io.chariox.slice.owner-machine-id', 'owner_machine_id')]:
        if labels.get(label) != options[name]:
            raise RuntimeError('slice container ownership mismatch')
    if first.get('HostConfig', {}).get('PidMode') not in ('', 'private'):
        raise RuntimeError('private slice PID namespace required')
    state = first.get('State', {})
    if state.get('Paused') or state.get('Restarting'):
        raise RuntimeError('slice is paused or restarting; overlay admission refused')
    if state.get('Running') is True and state.get('Status') == 'running' and state.get('Pid', 0) > 0:
        observation = json.loads(run(['exec', '-u', 'root', container_id, 'python3', '-c', CENSUS, profile, str(options['uid'])]))
        count = observation.get('profileProcessCount')
        if observation.get('disposition') not in ('clear', 'owned', 'unowned') or type(count) is not int or not 0 <= count <= 4096:
            raise RuntimeError('incomplete browser ownership census')
    elif state.get('Running') is False and state.get('Pid') == 0 and state.get('Status') in ('created', 'exited', 'dead'):
        observation = {'disposition': 'stopped', 'profileProcessCount': 0}
    else:
        raise RuntimeError('slice container state is ambiguous')
    after = json.loads(run(['inspect', container_id]))
    if len(after) != 1 or generation(after[0]) != generation(first) or run(['info', '--format', '{{.ID}}']).strip() != engine:
        raise RuntimeError('slice container or Docker engine changed during overlay admission')
    if observation['disposition'] == 'unowned':
        raise RuntimeError(f"legacy or unowned Chromium is still running in slice {options['slice_id']}; support overlay was not admitted. "
                           f"Use the normal kernel slice lifecycle: /slice stop {options['slice_id']}; wait until it is stopped, then /slice start {options['slice_id']}. "
                           'This read-only check did not stop provider turns, alter browser data, or create retirement receipts.')
    return {'schema': 'chariox.browser-overlay-admission.v1', 'engineId': engine, 'containerId': container_id,
            'sliceId': options['slice_id'], 'startedAt': state.get('StartedAt'), **observation}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--container', required=True)
    parser.add_argument('--slice-id', required=True)
    parser.add_argument('--owner-kernel-id', required=True)
    parser.add_argument('--owner-machine-id', required=True)
    parser.add_argument('--profile', help='explicit profile; otherwise use the existing container launcher environment')
    parser.add_argument('--uid', type=int, default=1001)
    parser.add_argument('--docker', default='docker')
    args = vars(parser.parse_args())
    docker = args.pop('docker')
    def run(command):
        result = subprocess.run([docker, *command], capture_output=True, text=True, timeout=15, check=False)
        if result.returncode or len(result.stdout) > 131072:
            raise RuntimeError('bounded Docker browser ownership observation failed')
        return result.stdout
    try:
        print(json.dumps(check(args, run)))
    except (RuntimeError, OSError, subprocess.TimeoutExpired, ValueError) as error:
        print(str(error) if isinstance(error, RuntimeError) else 'browser overlay ownership inspection failed', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
