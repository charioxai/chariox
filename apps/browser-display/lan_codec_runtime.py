"""MP-08/MP-10/MP-11: include the native worker's runtime-only codec roots."""
import os,re,subprocess
from pathlib import Path

def codec_roots(worker, environment=None):
 env=dict(os.environ if environment is None else environment)
 probe=subprocess.run([str(worker),'--display-native-codec-probe'],env=env,capture_output=True,text=True,check=True)
 names=re.findall(r'^MP-08/MP-10: native decoder (libavcodec\.so\.\d+)$',probe.stdout,re.M)
 if len(names)!=1:raise RuntimeError('MP-10: native worker decoder ABI missing')
 encoder=env.get('CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER')
 if encoder=='libx264':
  # The entry point is build-versioned; the binary contains that exact SONAME.
  literals=subprocess.check_output(['strings',str(worker)],text=True)
  versions=set(re.findall(r'libx264\.so\.\d+',literals))
  if len(versions)!=1:raise RuntimeError('MP-10: native worker x264 ABI missing')
  names+=list(versions)
 else:names+=['libopenh264.so.8']
 inventory=subprocess.check_output(['ldconfig','-p'],text=True)
 roots=[]
 for name in names:
  configured=env.get('CHARIOX_BROWSER_DISPLAY_OPENH264','') if name=='libopenh264.so.8' else ''
  if configured.startswith('/'):root=Path(configured)
  else:
   matches=re.findall(r'^\s*'+re.escape(name)+r'\s+.*=>\s+(\S+)$',inventory,re.M)
   if not matches:raise RuntimeError('MP-10: missing codec library '+name)
   root=Path(matches[0])
  if not root.is_file():raise RuntimeError('MP-10: missing codec library '+name)
  roots.append((root,name))
 return roots
