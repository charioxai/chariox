#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: three real-worker runs per compiler document.

The worker linear-memory cap is 512 MiB. Optional cgroup cap covers the
harness, bubblewrap and worker together, not just the guest's linear memory.
"""
import argparse,datetime,json,os,pathlib,subprocess,time

def main():
 p=argparse.ArgumentParser();p.add_argument('--state',required=True);p.add_argument('--root',required=True);p.add_argument('--evidence',required=True);p.add_argument('--cgroup-mib',type=int);a=p.parse_args()
 root=pathlib.Path(a.root);state=pathlib.Path(a.state);ev=pathlib.Path(a.evidence);rows=[]
 env={**os.environ,'SPIKE_ROOT':str(root/'spike'),'SPIKE_WORKER_ASYNC':str(root/'bin/worker'),'SPIKE_MEMORY_MB':'512','SPIKE_SLICE_MS':'30000','SPIKE_TEX_ORIGIN':'127.0.0.1:'+ (state/'proxy.port').read_text().strip()}
 for doc in ['hello','paper','beamer']:
  for n in range(1,4):
   stat=os.statvfs('/');free=stat.f_bavail*stat.f_frsize;total=stat.f_blocks*stat.f_frsize
   if free/total < .15:raise RuntimeError('b3 below shared 15% free disk floor; pause and resume this round')
   label=f'component-{doc}-{n}';command=[str(root/'bin/harness'),'wasm-rust-async','bwrap','tex',str(state/(doc+'.json'))]
   group=None
   if a.cgroup_mib:
    group=pathlib.Path('/sys/fs/cgroup')/f'texspike-{os.getpid()}-{doc}-{n}';group.mkdir();(group/'memory.max').write_text(str(a.cgroup_mib*1024*1024));(group/'memory.swap.max').write_text('0')
   def admit():
    if group:
     pid=os.getpid()
     if pid<=1:raise RuntimeError('invalid child PID')
     (group/'cgroup.procs').write_text(str(pid))
   start=time.monotonic()
   with (ev/(label+'.json')).open('w') as output,(ev/(label+'.err')).open('w') as error:
    r=subprocess.run(command,env=env,stdout=output,stderr=error,timeout=900,preexec_fn=admit if group else None)
   row={'document':doc,'run':n,'exit':r.returncode,'command':command,'wallMs':(time.monotonic()-start)*1000,'linearMemoryMiB':512,'cpuSliceMs':30000,'diskFreeBytes':free,'date':datetime.datetime.now(datetime.timezone.utc).isoformat()}
   if group:
    row['cgroupMiB']=a.cgroup_mib;row['cgroupPeakBytes']=int((group/'memory.peak').read_text());row['cgroupEvents']=(group/'memory.events').read_text();group.rmdir()
   data=(ev/(label+'.json')).read_text();row['report']=json.loads(data) if data else None
   rows.append(row);(ev/'component-results.json').write_text(json.dumps(rows,indent=2)+'\n')
   print(label,r.returncode,round(row['wallMs']),flush=True)
if __name__=='__main__':main()
