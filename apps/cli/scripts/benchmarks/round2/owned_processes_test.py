# MP-08/MP-10/MP-11: lane watchdog guards; no system or foreign signals.
import os,signal,unittest
from owned_processes import OwnedProcessTree,signal_owned_pid,identity

def row(pid,parent,group=20,start='100'):
 return {'pid':pid,'parent':parent,'group':group,'session':group,'start':start}
class Guards(unittest.TestCase):
 def test_invalid(self):
  for pid in [None,0,1,-1,-20,float('nan'),2.5,'20',True,os.getpid()]:
   with self.assertRaises(ValueError):OwnedProcessTree(pid)
 def test_mixed_group(self):
  calls=[];rows={20:row(20,os.getpid()),21:row(21,1)}
  tree=OwnedProcessTree(20,read=lambda:rows,send=lambda *args:calls.append(args))
  with self.assertRaises(ValueError):tree.signal(signal.SIGTERM)
  self.assertEqual(calls,[])
 def test_owned_orphan_and_reuse(self):
  calls=[];rows={20:row(20,os.getpid()),21:row(21,20)}
  tree=OwnedProcessTree(20,read=lambda:rows,send=lambda *args:calls.append(args))
  tree.signal(signal.SIGTERM);self.assertEqual([x[0] for x in calls],[21,20])
  rows={21:row(21,1)};tree.signal(signal.SIGKILL)
  rows={21:row(21,1,start='200')}
  with self.assertRaises(ValueError):tree.signal(signal.SIGKILL)
  self.assertEqual(len(calls),3)
 def test_recheck(self):
  calls=[]
  with self.assertRaises(ValueError):signal_owned_pid(20,signal.SIGTERM,identity(row(20,10)),read=lambda:{20:row(20,10,start='200')},send=lambda *args:calls.append(args))
  self.assertEqual(calls,[])
if __name__=='__main__':unittest.main()
