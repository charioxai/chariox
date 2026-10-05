"""MP-08/MP-10/MP-11: checked positive-PID signals using public proc metadata."""
import os, signal
from pathlib import Path

def safe_pid(pid):
 if type(pid) is not int or pid <= 1 or pid == os.getpid():
  raise ValueError('MP-10 unsafe cleanup PID')
 return pid

def processes():
 result={}
 for path in Path('/proc').iterdir():
  if not path.name.isdigit(): continue
  try:
   stat=(path/'stat').read_text();fields=stat[stat.rfind(')')+2:].split()
   result[int(path.name)]={'pid':int(path.name),'parent':int(fields[1]),'group':int(fields[2]),'session':int(fields[3]),'start':fields[19]}
  except (FileNotFoundError,ProcessLookupError):continue
 return result

def identity(row):return (row['start'],row['group'],row['session'])

def signal_owned_pid(pid, sig, expected, read=processes, send=os.kill):
 safe_pid(pid)
 if sig not in (signal.SIGTERM,signal.SIGKILL):raise ValueError('MP-10 unsafe signal')
 row=read().get(pid)
 if row is None:return
 if identity(row)!=expected:raise ValueError('MP-10 cleanup ownership changed')
 safe_pid(row['pid'])
 try:send(pid,sig)
 except ProcessLookupError:pass

class OwnedProcessTree:
 def __init__(self,pid,read=processes,send=os.kill):
  self.pid=safe_pid(pid);self.read=read;self.send=send;self.known={}
 def signal(self,sig):
  safe_pid(self.pid);rows=self.read();leader=rows.get(self.pid)
  if leader and (leader['group']!=self.pid or leader['session']!=self.pid or (identity(leader)!=self.known[self.pid] if self.pid in self.known else leader['parent']!=os.getpid())):
   raise ValueError('MP-10 cleanup leader ownership changed')
  def depth(row):
   current=row;seen=set();level=0
   while current and current['pid'] not in seen:
    if self.known.get(current['pid'])==identity(current) or current['pid']==self.pid and leader['parent']==os.getpid():return level
    seen.add(current['pid']);current=rows.get(current['parent']);level+=1
   raise ValueError('MP-10 cleanup descendant ownership missing')
  targets=[(depth(row),row) for row in rows.values() if row['group']==self.pid]
  for _,row in targets:safe_pid(row['pid']);self.known[row['pid']]=identity(row)
  for _,row in sorted(targets,key=lambda item:item[0],reverse=True):
   signal_owned_pid(row['pid'],sig,self.known[row['pid']],read=self.read,send=self.send)
