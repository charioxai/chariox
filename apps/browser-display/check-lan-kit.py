"""MP-11: prove fallback imports inside the kit with no host library files."""
import os,subprocess,sys,shutil
from pathlib import Path
kit=Path(sys.argv[1]).resolve()
if os.geteuid()!=0:raise SystemExit('MP-11: isolated kit check requires root for chroot')
script="""import sys,importlib.util
sys.path[:0]=['/pytools','/apps/kernel/slice-linux-docker/docker']
for name in ['kernel-browser-raster-damage','kernel-browser-stripes']:
 spec=importlib.util.spec_from_file_location(name,'/apps/kernel/slice-linux-docker/docker/'+name+'.py')
 module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
print('MP-11: isolated xxhash/capture/stripe imports PASS')
"""
subprocess.run([shutil.which('chroot') or '/usr/bin/chroot',str(kit),'/runtime/lib/ld-linux-x86-64.so.2','--library-path','/runtime/lib','/runtime/python/bin/python3','-c',script],env={'PYTHONHOME':'/runtime/python','PYTHONDONTWRITEBYTECODE':'1'},check=True)

# MP-08/MP-10/MP-11: dlopen dependencies are absent from ELF NEEDED/ldd.
# The worker opens both encoder and independent output decoder in the chroot.
if (kit/'runtime/native-worker').is_file():
 env={'CHARIOX_BROWSER_DISPLAY_OPENH264':'/runtime/lib/libopenh264.so.8','CHARIOX_BROWSER_DISPLAY_SOFTWARE':'1'}
 if os.environ.get('CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER')=='libx264':env['CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER']='libx264'
 subprocess.run([shutil.which('chroot') or '/usr/bin/chroot',str(kit),'/runtime/lib/ld-linux-x86-64.so.2','--library-path','/runtime/lib','/runtime/native-worker','--display-native-codec-probe'],env=env,check=True)
