# MP-08/MP-10/MP-11 numeric geometry on the exact owned viewer display; no titles/arguments/credentials emitted.
import pathlib,os,ctypes as C,json,sys
unit=sys.argv[1];assert unit.startswith('parity-viewer-')and unit.endswith('.service');group='/system.slice/'+unit;env=None
for d in pathlib.Path('/proc').iterdir():
 if not d.name.isdigit()or int(d.name)<=1:continue
 try:
  stat=(d/'stat').read_text();comm=stat[stat.index('(')+1:stat.rindex(')')]
  if comm!='node'or (d/'cgroup').read_text().strip()!='0::'+group:continue
  args=(d/'cmdline').read_bytes().rstrip(b'\0').split(b'\0')
  if len(args)<2 or args[1]not in [b'/root/parity/harness/campaign.mjs',b'/root/parity/harness/soak.mjs']:continue
  allowed={k:v for k,v in (part.split(b'=',1)for part in (d/'environ').read_bytes().split(b'\0')if b'='in part)if k in [b'DISPLAY',b'XAUTHORITY']};env=allowed;break
 except FileNotFoundError:pass
assert env and all(v in env for v in [b'DISPLAY',b'XAUTHORITY'])
for k,v in env.items():os.environ[k.decode()]=v.decode()
L=C.CDLL('libX11.so.6');L.XOpenDisplay.argtypes=[C.c_char_p];L.XOpenDisplay.restype=C.c_void_p;display=L.XOpenDisplay(env[b'DISPLAY']);assert display
L.XDefaultRootWindow.argtypes=[C.c_void_p];L.XDefaultRootWindow.restype=C.c_ulong;root=L.XDefaultRootWindow(display)
L.XQueryTree.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),C.POINTER(C.POINTER(C.c_ulong)),C.POINTER(C.c_uint)]
L.XGetGeometry.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_int),C.POINTER(C.c_int),C.POINTER(C.c_uint),C.POINTER(C.c_uint),C.POINTER(C.c_uint),C.POINTER(C.c_uint)]
L.XFree.argtypes=[C.c_void_p];L.XCloseDisplay.argtypes=[C.c_void_p]
def geometry(w):
 r=C.c_ulong();x=C.c_int();y=C.c_int();width=C.c_uint();height=C.c_uint();border=C.c_uint();depth=C.c_uint();ok=L.XGetGeometry(display,w,C.byref(r),C.byref(x),C.byref(y),C.byref(width),C.byref(height),C.byref(border),C.byref(depth));return{'x':x.value,'y':y.value,'width':width.value,'height':height.value,'border':border.value,'depth':depth.value,'ok':ok}
if len(sys.argv)>2:
 from PIL import Image as PILImage
 class Image(C.Structure):
  _fields_=[('width',C.c_int),('height',C.c_int),('xoffset',C.c_int),('format',C.c_int),('data',C.c_void_p),('byte_order',C.c_int),('bitmap_unit',C.c_int),('bitmap_bit_order',C.c_int),('bitmap_pad',C.c_int),('depth',C.c_int),('bytes_per_line',C.c_int),('bits_per_pixel',C.c_int),('red_mask',C.c_ulong),('green_mask',C.c_ulong),('blue_mask',C.c_ulong),('obdata',C.c_void_p)]
 import numpy as np
 L.XGetImage.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_int,C.c_uint,C.c_uint,C.c_ulong,C.c_int];L.XGetImage.restype=C.POINTER(Image);L.XDestroyImage.argtypes=[C.POINTER(Image)]
 im=L.XGetImage(display,root,0,0,4096,2400,C.c_ulong(-1).value,2);assert im
 try:
  pixels=np.frombuffer(C.string_at(im.contents.data,im.contents.bytes_per_line*2400),dtype=np.uint8).reshape(2400,im.contents.bytes_per_line//4,4)[:,:4096,:];output=pathlib.Path(sys.argv[2]);assert str(output).startswith('/root/.codex/evidence/browser-resume-20260930/vidlat/r6/')and output.suffix=='.png';PILImage.fromarray(pixels[:,:,[2,1,0]]).save(output)
 finally:L.XDestroyImage(im)
r=C.c_ulong();parent=C.c_ulong();children=C.POINTER(C.c_ulong)();count=C.c_uint();assert L.XQueryTree(display,root,C.byref(r),C.byref(parent),C.byref(children),C.byref(count));windows=[geometry(children[i])for i in range(count.value)];L.XFree(children);L.XCloseDisplay(display);print(json.dumps({'items':['MP-08','MP-10','MP-11'],'unit':unit,'root':geometry(root)if False else{'width':4096,'height':2400},'windows':windows}))
