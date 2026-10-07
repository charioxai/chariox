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
   receipt = {'source':source,'source_dirty':False,'protocol':454,'status':'PASS_LOCAL_COMPONENT','network':{'rtt':0},'latency':{'p95_ms':20},'samples':[{'mem_available_bytes':20*1024**3,'disk_free_bytes':30*1024**3}]}
   for version, expected in [(454,0),(446,1)]:
    receipt['protocol']=version
    (case/'results.json').write_text(json.dumps(receipt))
    result=subprocess.run(['python3',str(CHECKOUT/'apps/browser-display/report.py'),str(root),str(root/'report'),str(CHECKOUT)],capture_output=True,text=True)
    self.assertEqual(result.returncode,expected,result.stderr)
    if version==446:self.assertIn('unexpected protocol',result.stderr)

if __name__=='__main__':unittest.main()
