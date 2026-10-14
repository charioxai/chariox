# MP-08/MP-10/MP-11 observes actual headed viewer X11 pixels equally for S/V/M/C.
# Measurement only; never serves screenshots as the product's display transport.
import ctypes as C,os,time,json,sys,select,hashlib
import numpy as np
from PIL import Image as PILImage
lib=C.CDLL('libX11.so.6');lib.XOpenDisplay.argtypes=[C.c_char_p];lib.XOpenDisplay.restype=C.c_void_p;d=lib.XOpenDisplay(os.environ['DISPLAY'].encode());assert d,'own viewer X display'
lib.XDefaultRootWindow.argtypes=[C.c_void_p];lib.XDefaultRootWindow.restype=C.c_ulong;root=lib.XDefaultRootWindow(d)
class Image(C.Structure):
 _fields_=[('width',C.c_int),('height',C.c_int),('xoffset',C.c_int),('format',C.c_int),('data',C.c_void_p),('byte_order',C.c_int),('bitmap_unit',C.c_int),('bitmap_bit_order',C.c_int),('bitmap_pad',C.c_int),('depth',C.c_int),('bytes_per_line',C.c_int),('bits_per_pixel',C.c_int),('red_mask',C.c_ulong),('green_mask',C.c_ulong),('blue_mask',C.c_ulong),('obdata',C.c_void_p)]
lib.XGetImage.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_int,C.c_uint,C.c_uint,C.c_ulong,C.c_int];lib.XGetImage.restype=C.POINTER(Image);lib.XDestroyImage.argtypes=[C.POINTER(Image)];lib.XCloseDisplay.argtypes=[C.c_void_p]
def grab(roi):
 x,y,w,h=map(int,roi);assert x>=0 and y>=0 and 1<=w<=512 and 1<=h<=512
 im=lib.XGetImage(d,root,x,y,w,h,C.c_ulong(-1).value,2);assert im,'ROI outside own display'
 try:return np.frombuffer(C.string_at(im.contents.data,im.contents.bytes_per_line*h),dtype=np.uint8).reshape(h,im.contents.bytes_per_line//4,4)[:,:w,:]
 finally:lib.XDestroyImage(im)
def emit(v):print(json.dumps(v,separators=(',',':')),flush=True)
lib.XQueryTree.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),C.POINTER(C.POINTER(C.c_ulong)),C.POINTER(C.c_uint)]
lib.XGetGeometry.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_int),C.POINTER(C.c_int),C.POINTER(C.c_uint),C.POINTER(C.c_uint),C.POINTER(C.c_uint),C.POINTER(C.c_uint)];lib.XFree.argtypes=[C.c_void_p]
def physical_window():
 r=C.c_ulong();parent=C.c_ulong();children=C.POINTER(C.c_ulong)();count=C.c_uint();assert lib.XQueryTree(d,root,C.byref(r),C.byref(parent),C.byref(children),C.byref(count));found=[]
 try:
  for i in range(count.value):
   x=C.c_int();y=C.c_int();width=C.c_uint();height=C.c_uint();border=C.c_uint();depth=C.c_uint()
   if lib.XGetGeometry(d,children[i],C.byref(r),C.byref(x),C.byref(y),C.byref(width),C.byref(height),C.byref(border),C.byref(depth))and width.value>100 and height.value>100 and depth.value==24 and x.value>=0 and y.value>=0:found.append({'left':x.value,'top':y.value,'width':width.value,'height':height.value,'border':border.value})
 finally:lib.XFree(children)
 assert found,'own headed browser physical window absent'
 return max(found,key=lambda b:b['width']*b['height'])
def root_geometry():
 r=C.c_ulong();x=C.c_int();y=C.c_int();width=C.c_uint();height=C.c_uint();border=C.c_uint();depth=C.c_uint();assert lib.XGetGeometry(d,root,C.byref(r),C.byref(x),C.byref(y),C.byref(width),C.byref(height),C.byref(border),C.byref(depth));return {'width':width.value,'height':height.value}
active=None;buffer=b''
emit({'kind':'ready','items':['MP-08','MP-10','MP-11'],'method':'XGetImage of actual presented headed viewer, same sampler all paths'})
try:
 while True:
  if select.select([sys.stdin],[],[],.004)[0]:
   data=os.read(sys.stdin.fileno(),65536)
   if not data:break
   buffer+=data
   while b'\n' in buffer:
    line,buffer=buffer.split(b'\n',1);cmd=json.loads(line)
    if cmd['op']=='geometry':emit({'kind':'geometry','id':cmd['id'],'bounds':physical_window(),'root':root_geometry()})
    elif cmd['op']=='arm':
     roi=cmd['roi'];base=grab(roi);active={'id':cmd['id'],'roi':roi,'base':base,'at':time.time()*1000,'input':None,'changes':[],'luminance_samples':[],'last':base,'mode':cmd.get('mode','latency'),'threshold':cmd.get('threshold',15),'columns':cmd.get('columns',2),'caret_columns':cmd.get('caret_columns',2),'mask':np.zeros(base.shape[:2],dtype=bool)}
     if cmd.get('luminance'):
      lo,hi=cmd['luminance'];assert 0<=lo<hi<=255
      if cmd.get('first_target_change'):
       mean=float(base[:,:,:3].mean());direction=1 if (lo+hi)/2>mean else -1
       lo,hi=(min(255,mean+40),255)if direction>0 else(0,max(0,mean-40))
      active['luminance']=(lo,hi)
     if cmd.get('target_file'):
      file=cmd['target_file'];assert file.startswith('/root/.codex/evidence/browser-resume-20260930/vidlat/r6/') and file.endswith('.png');target=np.asarray(PILImage.open(file).convert('RGB'));active['target']=target
      # MP-03/MP-08/MP-10/MP-11 settled target supplies a known heading-stroke mask.
      luminance=target.mean(axis=2);background=float(np.median(luminance));dark=luminance<background-60;light=luminance>background+60;polarity=1 if int(light.sum())>=int(dark.sum())else -1;ink=light if polarity==1 else dark
      if cmd.get('glyph_target'):
       baseline_file=cmd['target_baseline_file'];assert baseline_file.startswith('/root/.codex/evidence/browser-resume-20260930/vidlat/r6/') and baseline_file.endswith('.png')
       empty=np.asarray(PILImage.open(baseline_file).convert('RGB'));assert empty.shape==target.shape
       ink=(empty.mean(axis=2)-target.mean(axis=2))>=40
       # Discard the moved caret; retain the real settled public glyph strokes.
       ink[:,np.sum(ink,axis=0)>=ink.shape[0]*.4]=False
       active['glyph_target']=True;polarity=-1
      active['target_ink_mask']=ink;active['target_ink_polarity']=polarity;active['target_background_mask']=np.abs(luminance-background)<10;active['target_ink_pixels']=int(ink.sum())
      def ink_fraction(pixels):
       rgb=pixels[:,:,[2,1,0]].mean(axis=2);bg=float(np.median(rgb[active['target_background_mask']]))
       return float(np.mean((rgb[ink]-bg)*active['target_ink_polarity']>24))if ink.any()else 0.0
      active['target_ink_baseline_fraction']=ink_fraction(base);active['target_ink_baseline_absent']=active['target_ink_baseline_fraction']<.10 and active['target_ink_pixels']>=(4 if active.get('glyph_target')else 20)
     emit({'kind':'armed','id':active['id'],'roi':roi,'at':active['at'],'baseline_mean':float(base[:,:,:3].mean()),'target_already_visible':bool('luminance' in active and active['luminance'][0]<=float(base[:,:,:3].mean())<=active['luminance'][1])})
    elif cmd['op']=='begin' and active and cmd['id']==active['id']:active['input']=cmd['at']
    elif cmd['op']=='end' and active and cmd['id']==active['id']:
     changes=[v for v in active['changes'] if v['at']>=cmd['start'] and v['at']<=cmd['end']];emit({'kind':'frames','id':active['id'],'changes':changes,'fps':len(changes)/((cmd['end']-cmd['start'])/1000)});active=None
    elif cmd['op']=='capture':
     pixels=grab(cmd['roi']);PILImage.fromarray(pixels[:,:,[2,1,0]]).save(cmd['file']);emit({'kind':'capture','id':cmd['id'],'roi':cmd['roi']})
    elif cmd['op']=='cancel':active=None
    elif cmd['op']=='close':raise SystemExit(0)
  if active:
   now=time.time()*1000;pixels=grab(active['roi']);base=active['base']
   # Ignore idle animation before dispatch; apply exactly the same rule to all paths.
   if active['input'] is None and now-active['at']<150:
    active['mask'] |= np.any(np.abs(pixels[:,:,:3].astype(np.int16)-base[:,:,:3].astype(np.int16))>24,axis=2)
   comparison=active['last'] if active['mode']=='frames' else base
   changed=np.any(np.abs(pixels[:,:,:3].astype(np.int16)-comparison[:,:,:3].astype(np.int16))>24,axis=2)&~active['mask']
   diff=int(changed.sum());columns=np.where(changed.any(axis=0))[0];rows=np.where(changed.any(axis=1))[0];caret_only=len(columns)<=active['caret_columns'] and len(rows)>=active['roi'][3]*.4
   if active['input'] is not None and now>=active['input'] and diff>=active['threshold'] and len(columns)>=active['columns'] and not caret_only and 'first_change_at'not in active:active['first_change_at']=now
   if 'target' in active and active['target_ink_baseline_absent'] and active['input'] is not None and now>=active['input'] and 'first_heading_ink_at'not in active:
    rgb=pixels[:,:,[2,1,0]].mean(axis=2);bg=float(np.median(rgb[active['target_background_mask']]));fraction=float(np.mean((rgb[active['target_ink_mask']]-bg)*active['target_ink_polarity']>24))
    if fraction>=.70:active['first_heading_ink_at']=now;active['first_heading_ink_fraction']=fraction
   target_ok='target'not in active or float(np.mean(np.any(np.abs(pixels[:,:,[2,1,0]].astype(np.int16)-active['target'].astype(np.int16))>24,axis=2)))<.025
   if active.get('glyph_target'):
    rgb=pixels[:,:,[2,1,0]].mean(axis=2);bg=float(np.median(rgb[active['target_background_mask']]));mask=active['target_ink_mask'];fraction=float(np.mean((rgb[mask]-bg)*active['target_ink_polarity']>24))if mask.any()else 0.0
    target_ok=active['target_ink_baseline_absent'] and fraction>=.70
   mean=float(pixels[:,:,:3].mean());
   if active['input'] is not None and len(active['luminance_samples'])<1500:active['luminance_samples'].append({'at':now,'mean':mean,'diff':diff})
   lo,hi=active.get('luminance',(0,255));target_ok=target_ok and lo<=mean<=hi
   if diff>=active['threshold'] and len(columns)>=active['columns'] and target_ok and not caret_only:
    if active['mode']=='frames':active['changes'].append({'at':now,'changed_pixels':diff});active['last']=pixels
    elif not active['changes'] or now-active['changes'][-1]['at']>3:active['changes'].append({'at':now,'changed_pixels':diff,'changed_columns':len(columns),'changed_rows':len(rows)})
   if active and active['mode']=='latency' and active['input'] is not None:
    candidate=next((v for v in active['changes'] if v['at']>=active['input']),None)
    if candidate:emit({'kind':'latency','id':active['id'],'input_at':active['input'],'presented_at':candidate['at'],'latency_ms':candidate['at']-active['input'],'changed_pixels':candidate['changed_pixels'],'changed_columns':candidate['changed_columns'],'changed_rows':candidate['changed_rows'],'caret_columns':active['caret_columns'],'first_qualifying_presented_change_at':active.get('first_change_at'),'first_change_latency_ms':active.get('first_change_at')-active['input']if active.get('first_change_at')else None,'roi':active['roi'],'luminance_samples':active['luminance_samples'],'qualified_mean':mean,'luminance_range':active.get('luminance'),'target_heading_ink':{'method':'known settled-heading foreground mask, >=70% known stroke pixels signed contrast>=24 levels against current heading background; real target chooses dark/light polarity; negative closed-panel stroke match<10%','target_pixels':active.get('target_ink_pixels'),'polarity':active.get('target_ink_polarity'),'baseline_fraction':active.get('target_ink_baseline_fraction'),'baseline_absent':active.get('target_ink_baseline_absent'),'first_fraction':active.get('first_heading_ink_fraction'),'first_presented_at':active.get('first_heading_ink_at'),'latency_ms':active.get('first_heading_ink_at')-active['input']if active.get('first_heading_ink_at')else None}if 'target' in active else None});active=None
    elif now-active['input']>5000:emit({'kind':'latency','id':active['id'],'input_at':active['input'],'latency_ms':None,'reason':'no qualifying presented pixel change within 5s','roi':active['roi']});active=None
finally:lib.XCloseDisplay(d)
