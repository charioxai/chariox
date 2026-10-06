"""MP-11: settle only projects recorded under this fresh Harbor job directory."""
import json
import re
import subprocess


def output(command):
    return subprocess.check_output(command,text=True).strip()


def settle_job(job, task):
    receipts=[]
    for trial in job.iterdir() if job.exists() else []:
        if not trial.is_dir() or not re.fullmatch(re.escape(task)+r'__[A-Za-z0-9]+',trial.name):
            continue
        project=(trial.name+'__env').lower()
        removed=[]
        ids=output(['docker','ps','-aq','--filter','label=com.docker.compose.project='+project]).splitlines()
        for cid in ids:
            actual=output(['docker','inspect',cid,'--format','{{.Id}} {{index .Config.Labels "com.docker.compose.project"}}'])
            if actual!=cid+' '+project:
                raise RuntimeError('MP-11: Harbor container identity mismatch')
            subprocess.run(['docker','stop','--time','10',cid],check=True,stdout=subprocess.DEVNULL)
            subprocess.run(['docker','rm',cid],check=True,stdout=subprocess.DEVNULL)
            removed.append(cid)
        networks=output(['docker','network','ls','-q','--filter','label=com.docker.compose.project='+project]).splitlines()
        for nid in networks:
            actual=json.loads(output(['docker','network','inspect',nid]))[0]
            if actual['Labels'].get('com.docker.compose.project')!=project or actual['Containers']:
                raise RuntimeError('MP-11: Harbor network has foreign attachments')
            subprocess.run(['docker','network','rm',nid],check=True,stdout=subprocess.DEVNULL)
        # No named data volume is configured by this adapter. Unknown volumes
        # are retained rather than recursively deleting credential-bearing state.
        volumes=output(['docker','volume','ls','-q','--filter','label=com.docker.compose.project='+project]).splitlines()
        receipts.append({'project':project,'removed_containers':removed,'removed_networks':networks,'retained_volumes':volumes})
    return receipts
