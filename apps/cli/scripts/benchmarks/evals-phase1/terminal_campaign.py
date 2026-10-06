#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: serial pinned Harbor tasks, fresh profiles and evidence."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import tomllib
from uuid import uuid4

from contract import validate_lock
from harbor_cleanup import settle_job
from turn import preflight, resource_sample, stop_owned


def command_output(command):
    return subprocess.check_output(command, text=True).strip()


def image_identity(tag):
    result = subprocess.run(['docker','image','inspect',tag,'--format','{{.Id}}'],capture_output=True,text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def trial_results(job):
    return [p for p in job.glob('*/result.json') if 'task_name' in json.loads(p.read_text())]


def resume_quota_campaign(campaign, identity):
    """MP-08 / MP-10 / MP-11: preserve admission attempt; completed official tasks stay fixed."""
    if any(campaign.get(k)!=v for k,v in identity.items()):
        raise ValueError('MP-08 / MP-10: resumed campaign identity differs')
    if campaign.get('status')!='blocked' or not campaign.get('tasks'):
        raise ValueError('MP-08 / MP-10: only a quota-paused campaign can resume')
    last=campaign['tasks'][-1]
    official=json.loads(Path(last['official_result']).read_text())
    measurement=((official.get('agent_result') or {}).get('metadata') or {}).get('chariox') or {}
    if measurement.get('status')!='quota_exhausted' or not measurement.get('cleanup_complete') or last['cleanup']['remaining_containers']:
        raise ValueError('MP-08 / MP-10 / MP-11: quota admission and complete cleanup required')
    campaign.setdefault('quota_attempts',[]).append(campaign['tasks'].pop())
    campaign['status']='running';campaign.pop('first_failing_task',None)
    return campaign


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--phase',choices=['smoke','full','admission'],required=True)
    for key in ['tasks','runtime-root','output','harbor','profile-path']:
        p.add_argument('--'+key,type=Path,required=True)
    p.add_argument('--source-commit',required=True)
    p.add_argument('--kernel-sha256',required=True)
    p.add_argument('--local-protocol',type=int,required=True)
    p.add_argument('--model',default='gpt-6.1-sol')
    p.add_argument('--admission-task',default='break-filter-js-from-html')
    p.add_argument('--resume',action='store_true')
    args=p.parse_args()
    scripts=Path(__file__).resolve().parent
    lock=json.loads((scripts/'inputs.lock.json').read_text());validate_lock(lock)
    if command_output(['git','-C',str(args.tasks),'rev-parse','HEAD']) != lock['terminal_bench_2']['revision'] or command_output(['git','-C',str(args.tasks),'status','--porcelain']):
        raise ValueError('MP-08 / MP-10: exact clean official task checkout required')
    if not args.output.is_absolute() or (args.output.exists() and not args.resume) or args.output.resolve().is_relative_to(scripts.parents[4]):
        raise ValueError('MP-11: new absolute external evidence directory required')
    preflight(str(args.runtime_root),args.source_commit,args.kernel_sha256,args.local_protocol)
    ids=lock['terminal_bench_2']['task_ids']
    selected=ids[:10] if args.phase=='smoke' else [args.admission_task] if args.phase=='admission' else ids
    args.output.mkdir(parents=True,mode=0o700,exist_ok=args.resume)
    campaign={'mp_items':['MP-08','MP-10','MP-11'],'benchmark':'terminal_bench_2','phase':args.phase,'task_ids':selected,
              'source_commit':args.source_commit,'kernel_sha256':args.kernel_sha256,'model':args.model,'harness_revision':lock['harbor']['revision'],
              'task_revision':lock['terminal_bench_2']['revision'],'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'started_at':time.time(),'tasks':[],'status':'running'}
    if args.resume:
        identity={key:campaign[key] for key in ['benchmark','phase','task_ids','source_commit','kernel_sha256','model','harness_revision','task_revision']}
        campaign=resume_quota_campaign(json.loads((args.output/'campaign.json').read_text()),identity)
    attempts=args.output/'campaign-attempts';attempts.mkdir(mode=0o700,exist_ok=True)
    (attempts/(uuid4().hex+'.json')).write_text(json.dumps({'mp_items':['MP-08','MP-10','MP-11'],'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'started_at':time.time(),'resume':args.resume},indent=2)+'\n')
    def save(): (args.output/'campaign.json').write_text(json.dumps(campaign,indent=2)+'\n')
    save()
    env={**os.environ,'PYTHONPATH':str(scripts),'PYTHONDONTWRITEBYTECODE':'1','HARBOR_DISABLE_TELEMETRY':'1'}
    try:
        for task in selected:
            if any(row['task_id']==task for row in campaign['tasks']):continue
            config=tomllib.loads((args.tasks/task/'task.toml').read_text())
            required=9*1024**3+(config['environment']['memory_mb']+1024)*1024**2
            # Wait for headroom, without running another task or holding a Rust slot.
            while resource_sample()['mem_available_bytes'] < required:
                time.sleep(5)
            image=config['environment']['docker_image']; before=image_identity(image)
            nonce=uuid4().hex[:12]; name='chariox-evals-'+args.phase+'-'+nonce
            job_dir=args.output/'jobs'/name
            options={'runtime_root':'/opt/chariox','profile_path':str(args.profile_path),'source_commit':args.source_commit,
                     'kernel_sha256':args.kernel_sha256,'local_protocol':args.local_protocol,'relay_binary':'/opt/chariox/bin/chariox-relay',
                     'timeout_seconds':max(60,int(config['agent']['timeout_sec'])-60)}
            job={'job_name':name,'jobs_dir':str(args.output/'jobs'),'n_concurrent_trials':1,'retry':{'max_retries':0},
                 'tasks':[{'path':str(args.tasks/task)}], 'agents':[{'import_path':'harbor_agent:CharioxAgent','model_name':args.model,'kwargs':options}],
                 'environment':{'type':'docker','delete':True,'mounts':[{'type':'bind','source':str(args.runtime_root),'target':'/opt/chariox','read_only':True},
                   {'type':'bind','source':str(args.profile_path),'target':str(args.profile_path)}]}}
            config_path=args.output/(task+'-job-'+nonce+'.json');config_path.write_text(json.dumps(job,indent=2)+'\n')
            invocation=[str(args.harbor),'run','--config',str(config_path)]
            start=time.monotonic();samples=[]
            cleanup=[]
            with (args.output/(task+'-'+nonce+'.log')).open('w') as log:
                process=subprocess.Popen(invocation,env=env,stdout=log,stderr=log)
                try:
                    while process.poll() is None:
                        sample=resource_sample();samples.append({'time':time.time(),**sample})
                        if sample['mem_available_bytes']<10*1024**3 or sample['root_free_bytes']<15*1024**3:
                            raise RuntimeError('MP-11: measured safety reserve reached')
                        time.sleep(2)
                finally:
                    stop_owned(process)
                    cleanup=settle_job(job_dir,task)
                    (args.output/(task+'-'+nonce+'-cleanup.json')).write_text(json.dumps(cleanup,indent=2)+'\n')
                exit_code=process.wait()
            matches=trial_results(job_dir)
            if len(matches)!=1:raise ValueError('MP-08 / MP-10: exactly one official task receipt required')
            official=json.loads(matches[0].read_text())
            trial=official['trial_name']
            project=(trial+'__env').lower()
            remaining=command_output(['docker','ps','-aq','--filter','label=com.docker.compose.project='+project]).splitlines()
            if remaining:raise RuntimeError('MP-11: owned Harbor task container remains')
            after=image_identity(image); removed=False
            if before is None and after is not None:
                used=command_output(['docker','ps','-aq','--filter','ancestor='+after])
                if not used and image_identity(image)==after:
                    subprocess.run(['docker','image','rm',image],check=True,stdout=subprocess.DEVNULL);removed=True
            metadata=((official.get('agent_result') or {}).get('metadata') or {}).get('chariox') or {}
            row={'task_id':task,'official_result':str(matches[0]),'job_exit_code':exit_code,
                 'command':invocation,'wall_time_seconds':time.monotonic()-start,'resources':samples,
                 'cleanup':{'manual_settlement':cleanup,'remaining_containers':remaining,'image_before':before,'image_after':after,'new_unused_image_tag_removed':removed}}
            campaign['tasks'].append(row);save()
            rewards=(official.get('verifier_result') or {}).get('rewards')
            print('MP-08 / MP-10:',args.phase,task,'reward',rewards,'settlement',metadata.get('status'),flush=True)
            if any(r['removed_containers'] or r['retained_volumes'] for r in cleanup) or official.get('exception_info') or not metadata.get('cleanup_complete') or metadata.get('status')!='completed' or not metadata.get('tui_usage_visible'):
                campaign['status']='blocked'; campaign['first_failing_task']=task;break
        else: campaign['status']='completed'
    except BaseException as error:
        campaign['status']='blocked';campaign['failure_kind']=type(error).__name__
        raise
    finally:
        campaign['finished_at']=time.time();save()
    return 0 if campaign['status']=='completed' else 1


if __name__=='__main__':raise SystemExit(main())
