"""MD-DISPLAY-02/04: retain failed cases and separate source/binary provenance."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
root, output, checkout = map(Path, sys.argv[1:])
campaign=json.loads((root/'campaign.json').read_text())
kit=json.loads((checkout/'KIT_MANIFEST.json').read_text()) if (checkout/'KIT_MANIFEST.json').exists() else None
if kit and kit['source']!=campaign['source']:raise ValueError('MD-DISPLAY: mixed kit source')
def source_bytes(relative):
 if kit:
  expected=next((f['sha256'] for f in kit['files'] if f['path']==relative),None)
  contents=(checkout/relative).read_bytes()
  if not expected or hashlib.sha256(contents).hexdigest()!=expected:raise ValueError('MD-DISPLAY: changed kit source '+relative)
  return contents
 return subprocess.check_output(['git','show',f"{campaign['source']}:{relative}"],cwd=checkout)
identity=json.loads((root/'binary.json').read_text()) if (root/'binary.json').exists() else None
digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
report={'item':'MD-DISPLAY-02/04','execution_source':campaign['source'],'kernel_binary':identity,
 'campaign_exit_code':campaign.get('exit_code'),'status':'RED_FUNCTIONAL','cases':[],
 'limits':['Software rAF/canvas proxy, not physical photons.','Only viewer leg WAN shaped; no hosted/native-OS proof.',
 'Live PSNR includes temporal drift.','Event Mbps excludes requests/TLS/unpaced diagnostics.',
 'Controller overrides and kernel binary provenance are separate; no inferred embedded-file coverage.']}
for case in campaign['cases']:
 name=f"{case['profile']}-{case['workload']}-{case['bitrate']}"
 receipt=root/name/'results.json'
 row={'name':name,'profile':case['profile'],'workload':case['workload'],'budget_mbps':case['bitrate']/1e6,
  'exit_code':case.get('code'),'signal':case.get('signal'),'namespace_cleanup':case.get('cleanup')}
 report['cases'].append(row)
 if not receipt.exists():row.update(functional_pass=False,error='missing receipt');continue
 data=json.loads(receipt.read_text());row.update(receipt=str(receipt),receipt_sha256=digest(receipt),status=data['status'],error=data.get('error'),cleanup=data.get('cleanup'))
 if data['source']!=campaign['source'] or data['source_dirty']:raise ValueError('MD-DISPLAY: mixed/dirty source')
 if data.get('protocol') is None:row.update(functional_pass=False);continue
 if data['protocol']!=441:raise ValueError('MD-DISPLAY: unexpected protocol')
 if identity and data['binary']['copied_sha256']!=identity['sha256']:raise ValueError('MD-DISPLAY: mixed binary')
 # Bind copied controller files to the exact Git blobs actually executed.
 assets=data.get('controller_assets',[])
 if isinstance(assets,dict):assets=assets.get('files',[])
 for asset in assets:
  content=source_bytes(f"apps/kernel/slice-linux-docker/docker/{asset['name']}")
  if hashlib.sha256(content).hexdigest()!=asset['sha256']:raise ValueError('MD-DISPLAY: controller source mismatch '+asset['name'])
 for asset in data.get('client_assets',[]):
  content=source_bytes(f"apps/browser-display/{asset['name']}")
  if hashlib.sha256(content).hexdigest()!=asset['sha256']:raise ValueError('MD-DISPLAY: client source mismatch '+asset['name'])
 row['client_assets']=data.get('client_assets',[])
 row['controller_assets']=assets
 row['codec']=data.get('codec');row['codec_provenance']=data.get('codec_provenance','Historical receipt used a stale literal codec label; actual negotiation was not recorded.')
 row['latency']=data.get('latency');row['exact_settled']=data.get('settled',{}).get('fidelity',{}).get('lossless',False)
 row['functional_pass']=case.get('code')==0 and not case.get('signal') and not case.get('remaining_namespace_pids') and data['status']=='PASS_LOCAL_COMPONENT'
 row['latency_target_p95_ms']=data['network']['rtt']+50
 row['latency_pass']=bool(row['latency']) and row['latency']['p95_ms']<=row['latency_target_p95_ms']
 row['click_stages']={k:{n:v[n] for n in ('n','p50_ms','p95_ms')} for k,v in data.get('stage_breakdown',{}).get('stages',{}).items()}
 motion=data.get('motion');row['motion']=None
 if motion:
  row['motion']={k:motion.get(k) for k in ('effective_fps','event_mbps','application_mbps','settle_ms','settle_present_ms','wheel_stats')}
  row['motion'].update(exact=motion['settled_fidelity']['lossless'],fps_pass=motion['effective_fps']>=30,settle_pass=bool(data['network']['rtt']) or motion.get('settle_present_ms',motion['settle_ms'])<=500,
   live_psnr_db=[p['psnr_db'] for p in motion.get('live_pairs',[])])
  spans={}
  if motion['samples']:
   start,end=motion['samples'][0]['presented_ms'],motion['samples'][-1]['presented_ms']
   for span in data['host_timings']+data['kernel_timings']+data['client_timings']:
    if start<=span['started_ms']<=span['ended_ms']<=end:spans.setdefault(span['stage'],[]).append(span['duration_ms'])
  row['motion_stages']={}
  for k,v in spans.items():
   v.sort();row['motion_stages'][k]={'n':len(v),'p50_ms':v[(len(v)-1)//2],'p95_ms':v[int((len(v)-1)*.95)]}
 row['fixture_statistics']=data.get('fixture_statistics',[])
 row['cpu_percent']=data.get('observed_owned_cpu_percent')
 row['min_mem_available_gib']=min(s['mem_available_bytes'] for s in data['samples'])/1024**3
 row['min_disk_free_gib']=min(s['disk_free_bytes'] for s in data['samples'])/1024**3
 row['artifacts']={p.name:digest(p) for p in receipt.parent.glob('*.png')}
report['functional_cases_passed']=sum(r.get('functional_pass',False) for r in report['cases'])
report['latency_cases_passed']=sum(r.get('latency_pass',False) for r in report['cases'])
report['motion_cases_passed']=sum(bool(r.get('motion') and r['motion']['fps_pass']) for r in report['cases'])
functional=report['functional_cases_passed']==len(report['cases']) and campaign.get('exit_code')==0
performance=functional and all(r['latency_pass'] and (not r['motion'] or r['motion']['fps_pass'] and r['motion']['settle_pass']) for r in report['cases'])
report['status']='PASS_PERFORMANCE_COMPONENT' if performance else 'RED_PERFORMANCE' if functional else 'RED_FUNCTIONAL'
output.mkdir(parents=True,exist_ok=True);(output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({k:report[k] for k in ('item','status','execution_source','functional_cases_passed','latency_cases_passed','motion_cases_passed')}))
sys.exit(0 if performance else 1)
