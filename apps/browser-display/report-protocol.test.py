"""MP-08/MP-10/MP-11: aggregation accepts bound versions and rejects mismatches."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

CHECKOUT = Path(__file__).resolve().parents[2]

class ProtocolReport(unittest.TestCase):
 def test_current_protocol_and_mismatched_binary(self):
  source = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=CHECKOUT, text=True).strip()
  with tempfile.TemporaryDirectory(prefix='chariox-mp10-report-') as temporary:
   root = Path(temporary)
   case = root/'local-docs-8000000'
   case.mkdir()
   (root/'campaign.json').write_text(json.dumps({'source':source,'exit_code':0,'cases':[{'profile':'local','workload':'docs','bitrate':8000000,'code':0}]}))
   receipt = {'source':source,'source_dirty':False,'protocol':466,'status':'PASS_LOCAL_COMPONENT','network':{'rtt':0},'dpr':1,'latency':{'p95_ms':20},'type_latency':{'p95_ms':20},'type_cpu':{'cores':{'pipeline':.3}},'samples':[{'mem_available_bytes':20*1024**3,'disk_free_bytes':30*1024**3}]}
   for version, expected in [(466,0),(465,1)]:
    receipt['protocol']=version
    (case/'results.json').write_text(json.dumps(receipt))
    result=subprocess.run(['python3',str(CHECKOUT/'apps/browser-display/report.py'),str(root),str(root/'report'),str(CHECKOUT)],capture_output=True,text=True)
    self.assertEqual(result.returncode,expected,result.stderr)
    if version==465:self.assertIn('unexpected protocol',result.stderr)
 def test_mp10_performance_cannot_pass_typing_cpu_missing_or_slow_refinement(self):
  source=subprocess.check_output(['git','rev-parse','HEAD'],cwd=CHECKOUT,text=True).strip()
  with tempfile.TemporaryDirectory(prefix='chariox-mp10-report-') as temporary:
   root=Path(temporary);case=root/'local-wheel60-8000000';case.mkdir()
   (root/'campaign.json').write_text(json.dumps({'source':source,'exit_code':0,'cases':[{'profile':'local','workload':'wheel60','bitrate':8000000,'code':0}]}))
   base={'source':source,'source_dirty':False,'protocol':466,'status':'PASS_LOCAL_COMPONENT','network':{'rtt':0},'dpr':1,'latency':{'p95_ms':20},'type_latency':{'p95_ms':20},'motion':{'samples':[],'effective_fps':59.9,'settle_present_ms':250,'settle_ms':250,'settled_fidelity':{'lossless':True},'cpu':{'cores':{'pipeline':.5}}},'samples':[{'mem_available_bytes':20*1024**3,'disk_free_bytes':30*1024**3}]}
   for seam in ['green','type','missing_type','cpu','missing_cpu','refinement','cadence','motion_type','missing_motion_type']:
    receipt=json.loads(json.dumps(base))
    if seam=='type':receipt['type_latency']['p95_ms']=200
    if seam=='missing_type':receipt.pop('type_latency')
    if seam=='cpu':receipt['motion']['cpu']['cores']['pipeline']=.7
    if seam=='missing_cpu':receipt['motion'].pop('cpu')
    if seam=='refinement':receipt['motion']['settle_present_ms']=300
    if seam=='cadence':receipt['motion']['effective_fps']=54
    if seam=='motion_type':receipt['motion']['typing']={'condition':'concurrent active motion','latency':{'n':1,'p95_ms':200}}
    if seam=='missing_motion_type':receipt['motion']['typing']={'condition':'concurrent active motion'}
    (case/'results.json').write_text(json.dumps(receipt))
    result=subprocess.run(['python3',str(CHECKOUT/'apps/browser-display/report.py'),str(root),str(root/'report'),str(CHECKOUT)],capture_output=True,text=True)
    self.assertEqual(result.returncode,0 if seam=='green' else 1,(seam,result.stderr))
    verdict=json.loads((root/'report/report.json').read_text())
    self.assertEqual(verdict['status'],'PASS_PERFORMANCE_COMPONENT' if seam=='green' else 'RED_PERFORMANCE',seam)

class DensitySettleReport(unittest.TestCase):
 def test_mp10_density_settle_limits_are_strict(self):
  source=subprocess.check_output(['git','rev-parse','HEAD'],cwd=CHECKOUT,text=True).strip()
  with tempfile.TemporaryDirectory(prefix='chariox-mp10-density-') as temporary:
   root=Path(temporary);case=root/'local-scroll60-8000000';case.mkdir()
   (root/'campaign.json').write_text(json.dumps({'source':source,'exit_code':0,'cases':[{'profile':'local','workload':'scroll60','bitrate':8000000,'code':0}]}))
   base={'source':source,'source_dirty':False,'protocol':466,'status':'PASS_LOCAL_COMPONENT','network':{'rtt':0},'latency':{'p95_ms':20},'type_latency':{'p95_ms':20},'motion':{'samples':[],'effective_fps':60,'settled_fidelity':{'lossless':True},'cpu':{'cores':{'pipeline':.5}}},'samples':[{'mem_available_bytes':20*1024**3,'disk_free_bytes':30*1024**3}]}
   for dpr,settle,code in [(1,299,0),(1,300,1),(2,499,0),(2,500,1)]:
    receipt=json.loads(json.dumps(base));receipt['dpr']=dpr;receipt['motion']['settle_present_ms']=settle
    (case/'results.json').write_text(json.dumps(receipt))
    result=subprocess.run(['python3',str(CHECKOUT/'apps/browser-display/report.py'),str(root),str(root/'report'),str(CHECKOUT)],capture_output=True,text=True)
    self.assertEqual(result.returncode,code,(dpr,settle,result.stderr))
    verdict=json.loads((root/'report/report.json').read_text())
    self.assertEqual(verdict['cases'][0]['motion']['settle_target_ms'],500 if dpr==2 else 300)

if __name__=='__main__':unittest.main()
