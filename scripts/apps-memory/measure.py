#!/usr/bin/env python3
"""MP-08 / MP-10: Linux App-view memory drill; existing opt-in kernel fixture."""
import argparse, json, math, os, pathlib, shutil, signal, subprocess, tempfile, time

TEST = 'runtime::router::tests::user_app_views::memory_budget::app_view_memory_budget_drill'
CLASSES = ('browser', 'renderers', 'GPU', 'controller', 'utility', 'zygote', 'kernel', 'worker', 'other')

def slice_labels(ownership):
    # MP-11: record IDs repeat in fresh homes; require the entire product owner tuple.
    return {f'io.chariox.slice.{label}': ownership[field] for label, field in
            [('id','slice_id'), ('owner-kernel-id','owner_kernel_id'), ('runtime-name','runtime_name')]}

def owned_slice_resource(labels, ownership):
    return bool(labels) and all(labels.get(key) == value for key, value in slice_labels(ownership).items())

def slice_listing(kind, ownership):
    command = ['docker','ps','-aq'] if kind == 'container' else ['docker','volume','ls','-q']
    return command + [arg for key,value in slice_labels(ownership).items()
                      for arg in ['--filter',f'label={key}={value}']]

def process(pid):
    try:
        stat = pathlib.Path(f'/proc/{pid}/stat').read_text().split(') ', 1)[1].split()
        cmd = pathlib.Path(f'/proc/{pid}/cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace')
        return {'pid': pid, 'ppid': int(stat[1]), 'start': stat[19], 'cmd': cmd}
    except (OSError, ValueError, IndexError):
        return None

def process_tree(root, retained, marker=None):
    rows = [row for entry in pathlib.Path('/proc').iterdir() if entry.name.isdecimal()
            and (row := process(int(entry.name)))]
    # MP-11: a reused root PID must never admit a foreign descendant tree.
    selected = set()
    selected.update(row['pid'] for row in rows if retained.get(row['pid']) == row['start']
                    or (marker and (marker + '/') in row['cmd']
                        and row['cmd'].split()[0].endswith('/chrome_crashpad_handler')))
    for _ in rows:
        old = len(selected)
        selected.update(row['pid'] for row in rows if row['ppid'] in selected)
        if len(selected) == old: break
    chosen = [r for r in rows if r['pid'] in selected and r['pid'] > 1]
    retained.update({r['pid']: r['start'] for r in chosen})
    return chosen

def category(cmd):
    if '--type=renderer' in cmd: return 'renderers'
    if '--type=gpu-process' in cmd: return 'GPU'
    if '--type=utility' in cmd: return 'utility'
    if '--type=zygote' in cmd: return 'zygote'
    if ('/chromium' in cmd or '/chrome' in cmd) and '--user-data-dir=' in cmd: return 'browser'
    if 'browser-controller.mjs' in cmd or 'kernel-browser-host.mjs' in cmd: return 'controller'
    if 'chariox_kernel-' in cmd or 'chariox-kernel' in cmd: return 'kernel'
    if 'sdk_tool' in cmd: return 'worker'
    return 'other'

def memory(pid):
    rss = pss = None
    try:
        for line in pathlib.Path(f'/proc/{pid}/status').read_text().splitlines():
            if line.startswith('VmRSS:'): rss = int(line.split()[1])
    except OSError: pass
    try:
        for line in pathlib.Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
            if line.startswith('Pss:'): pss = int(line.split()[1])
    except OSError: pass
    return rss, pss

def protection(pid):
    result = {}
    try:
        for line in pathlib.Path(f'/proc/{pid}/status').read_text().splitlines():
            name, _, value = line.partition(':')
            if name in ('NoNewPrivs', 'Seccomp', 'Seccomp_filters'):
                result[name] = int(value.strip())
            elif name == 'NSpid':
                result['pid_namespace_depth'] = len(value.split())
    except (OSError, ValueError): pass
    return result


def stop_owned(pid, identity, sig):
    if not isinstance(pid, int) or pid <= 1: raise ValueError('MP-11: unsafe signal target')
    row = process(pid)
    if row and row['start'] == identity: os.kill(pid, sig)

def resource(check=True):
    fields = pathlib.Path('/proc/meminfo').read_text().splitlines()
    available = int(next(v for v in fields if v.startswith('MemAvailable:')).split()[1])
    disk = shutil.disk_usage('/').free
    if check and (available < 10 * 1024**2 or disk < 12 * 1024**3):
        raise RuntimeError('MP-08 / MP-10: resource buffer reached (floor9GiB RAM/10GiB disk)')
    return {'mem_available_kib': available, 'disk_free_bytes': disk}

def quantile(values, q):
    if not values: return None
    values = sorted(values)
    return round(values[max(0, math.ceil(len(values) * q) - 1)] / 1024, 3)

def summarize(samples):
    result = []
    groups = sorted({(s['topology'], s['views'], s['activity']) for s in samples
                     if s['activity'] in ('idle', 'interacting', 'mirror_idle', 'mirror_interacting')})
    for key in groups:
        cohort = [s for s in samples if (s['topology'], s['views'], s['activity']) == key]
        for klass in (*CLASSES, 'browser_total'):
            classes = {'browser','renderers','GPU','utility','zygote'} if klass == 'browser_total' else {klass}
            rss, pss, counts, individual_rss, individual_pss = [], [], [], [], []
            for s in cohort:
                rows = [p for p in s['processes'] if p['class'] in classes]
                counts.append(len(rows))
                individual_rss.extend(p['rss_kib'] for p in rows if p['rss_kib'] is not None)
                individual_pss.extend(p['pss_kib'] for p in rows if p['pss_kib'] is not None)
                if all(p['rss_kib'] is not None for p in rows): rss.append(sum(p['rss_kib'] for p in rows))
                if all(p['pss_kib'] is not None for p in rows): pss.append(sum(p['pss_kib'] for p in rows))
            result.append({'topology': key[0], 'views':key[1], 'activity':key[2], 'class':klass,
                'samples':len(cohort), 'process_count_min':min(counts), 'process_count_max':max(counts),
                'per_process_rss_p50_mib':quantile(individual_rss,.5), 'per_process_rss_p95_mib':quantile(individual_rss,.95),
                'per_process_pss_p50_mib':quantile(individual_pss,.5), 'per_process_pss_p95_mib':quantile(individual_pss,.95),
                'rss_samples':len(rss), 'pss_samples':len(pss),
                'rss_p50_mib':quantile(rss,.5), 'rss_p95_mib':quantile(rss,.95),
                'rss_max_mib':quantile(rss,1),
                'pss_p50_mib':quantile(pss,.5), 'pss_p95_mib':quantile(pss,.95),
                'pss_max_mib':quantile(pss,1)})
    return result

def run(args):
    output = pathlib.Path(args.output).resolve(); output.mkdir(parents=True, exist_ok=True)
    root = pathlib.Path(tempfile.mkdtemp(prefix=f'{args.topology}-', dir='/root/.chariox/dev/appsbudget'))
    owned = {}; samples = []; container = None; child = display = None; exit_code = None
    receipt = {'mp_items':['MP-08','MP-10'], 'test':TEST, 'root':str(root), 'started':time.time(),
               'topology':args.topology, 'source':subprocess.check_output(['git','rev-parse','HEAD'], text=True).strip(),
               'source_diff_sha256':__import__('hashlib').sha256(subprocess.check_output(['git','diff','HEAD'])).hexdigest(),
               'binary_sha256':__import__('hashlib').file_digest(open(args.binary,'rb'),'sha256').hexdigest(),
               'image':args.image, 'cleanup':False}
    def save():
        (output/'receipt.json').write_text(json.dumps(receipt, indent=2))
        (output/'summary.json').write_text(json.dumps(summarize(samples), indent=2))
    save()
    try:
        resource()
        env = {'PATH':'/usr/local/bin:/usr/bin:/bin', 'HOME':str(root/'home'), 'TMPDIR':str(root),
            'CHARIOX_HOME':str(root/'chariox'), 'CHARIOX_APPS_MEMORY_ROOT':str(root),
            'CHARIOX_APPS_MEMORY_TOPOLOGY':args.topology, 'CHARIOX_APPS_MEMORY_IMAGE':args.image,
            'CHARIOX_SLICE_DOCKER_PROVISIONER':str(pathlib.Path('apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh').resolve()),
            'RUST_BACKTRACE':'0', 'RUST_TEST_THREADS':'1', 'RUST_MIN_STACK':str(32*1024**2),
            'TOKIO_WORKER_THREADS':'2', 'CHARIOX_KERNEL_BROWSER_MIRROR':'1', 'CHARIOX_KERNEL_HOST':'127.0.0.1', 'CHARIOX_KERNEL_PORT':'49901',
            'CHARIOX_BROWSER_CONTROLLER_NODE':'/usr/bin/node'}
        (root/'home').mkdir(); (root/'chariox').mkdir(); (root/'tmp').mkdir()
        # MP-11: scope the existing fixed worker's hard-coded /tmp to this run.
        namespace = ['unshare','--mount','--propagation','private','bash','-c',
            'mount --bind "$1" /tmp || exit; shift; exec "$@"',
            'appsbudget-private-tmp',str(root/'tmp')]
        command = [args.binary, '--exact', '--ignored', TEST, '--nocapture']
        if args.topology == 'host':
            for p in [root,root/'home',root/'chariox',root/'tmp']: os.chown(p,65534,65534)
            display = subprocess.Popen([*namespace,'setpriv','--reuid=65534','--regid=65534','--clear-groups','Xvfb',
                '-displayfd','1','-screen','0','1280x800x24','-nolisten','tcp'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            assert display.pid > 1
            owned[display.pid] = process(display.pid)['start']
            env['DISPLAY'] = ':' + display.stdout.readline().decode().strip()
            env['CHARIOX_KERNEL_BROWSER_EXECUTABLE'] = args.chrome
            env['LD_LIBRARY_PATH'] = args.library_path
            command = ['setpriv','--reuid=65534','--regid=65534','--clear-groups', *command]
        command = [*namespace, *command]
        with (output/'kernel.log').open('wb') as log, (output/'samples.jsonl').open('w') as raw:
            child = subprocess.Popen(command, env=env, stdout=log, stderr=log)
            assert child.pid > 1
            identity = process(child.pid)
            if identity: owned[child.pid] = identity['start']
            receipt['pid'] = child.pid; receipt['command']=command; save()
            deadline = time.monotonic() + 600
            while child.poll() is None:
                if time.monotonic() > deadline: raise TimeoutError('MP-08 / MP-10 drill timeout')
                resources = resource()
                rows = process_tree(child.pid, owned, str(root))
                if (root/'slice-ownership.json').exists() and not container:
                    ownership = json.loads((root/'slice-ownership.json').read_text())
                    receipt['slice_ownership'] = ownership
                    names = subprocess.check_output(slice_listing('container', ownership),text=True).split()
                    if len(names) == 1:
                        container = names[0]; receipt['container'] = container; save()
                if container:
                    pids = subprocess.run(['docker','top',container,'-eo','pid'], capture_output=True,text=True)
                    if pids.returncode == 0:
                        rows += [r for v in pids.stdout.split()[1:] if v.isdecimal() and (r := process(int(v)))]
                try: phase = json.loads((root/'phase.json').read_text())
                except (OSError,json.JSONDecodeError): phase = {'topology':args.topology,'views':0,'activity':'startup'}
                processes=[]
                for r in {r['pid']:r for r in rows}.values():
                    rss,pss=memory(r['pid'])
                    processes.append({'pid':r['pid'],'start':r['start'],'class':category(r['cmd']), 'rss_kib':rss,'pss_kib':pss, 'protection':protection(r['pid']),
                        'sandbox_disabled_flag': any(flag in r['cmd'] for flag in ['--no-sandbox','--disable-seccomp-filter-sandbox','--disable-namespace-sandbox'])})
                sample={**phase,'sample_at':time.time(), **resources,'processes':processes}
                samples.append(sample);raw.write(json.dumps(sample)+'\n');raw.flush()
                time.sleep(.1)
            exit_code=child.returncode
            receipt['exit_code']=exit_code
            if exit_code: receipt['result']='RED'
            else: receipt['result']='PASS'
    except BaseException as error:
        receipt['result']='RED';receipt['error']=str(error)
        raise
    finally:
        validation = root/'validation.json'
        if validation.exists():
            shutil.copyfile(validation, output/'validation.json')
        if (root/'slice-ownership.json').exists():
            receipt['slice_ownership'] = json.loads((root/'slice-ownership.json').read_text())
        if child: process_tree(child.pid, owned, str(root))
        # Exact start identities discovered only in this run's descendant tree.
        for sig in (signal.SIGTERM,signal.SIGKILL):
            for pid,identity in reversed(list(owned.items())):
                try: stop_owned(pid,identity,sig)
                except ProcessLookupError: pass
            time.sleep(.5)
        if child: child.wait(timeout=10)
        if display: display.wait(timeout=10)
        if receipt.get('slice_ownership'):
            ownership=receipt['slice_ownership']
            # Query exact product labels; never prune or remove a foreign resource.
            for kind in ['container','volume']:
                listing = slice_listing(kind, ownership)
                for name in subprocess.check_output(listing,text=True).split():
                    data=json.loads(subprocess.check_output(['docker',kind,'inspect',name]))[0]
                    labels=data['Config']['Labels'] if kind=='container' else data['Labels']
                    assert owned_slice_resource(labels, ownership), 'MP-11: foreign Docker resource refused'
                    subprocess.run(['docker','rm','-f',name] if kind=='container' else ['docker','volume','rm',name],check=True,stdout=subprocess.DEVNULL)
        alive=[pid for pid,identity in owned.items() if (r:=process(pid)) and r['start']==identity and r['cmd']]
        receipt['remaining_owned_pids']=alive
        receipt['finished']=time.time();receipt['resources_after']=resource(check=False)
        receipt['cleanup']=not alive
        # Runtime-generated identities are confined to this known disposable root.
        shutil.rmtree(root)
        save()
    print(json.dumps({'mp_items':['MP-08','MP-10'],'result':receipt['result'],'exit':exit_code,'cleanup':receipt['cleanup']}))
    return 0 if exit_code==0 and receipt['cleanup'] else 1

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--binary',required=True);p.add_argument('--output',required=True)
    p.add_argument('--topology',choices=['host','slice'],required=True);p.add_argument('--image',required=True)
    p.add_argument('--chrome');p.add_argument('--library-path');args=p.parse_args()
    raise SystemExit(run(args))
