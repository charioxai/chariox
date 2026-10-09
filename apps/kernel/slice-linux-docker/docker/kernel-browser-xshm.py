"""MD-DISPLAY-02/04: XDamage-triggered XShm browser-window readback.
Private binary pipe. Caller proves display ownership; owned desktop mode masks
before pixels cross the pipe or enter hashing, damage, or codec state.
"""
import ctypes as c
import ctypes.util
import json
import os
import select
import struct
import sys
import time
import importlib.util
import mmap
from pathlib import Path
spec=importlib.util.spec_from_file_location('raster_damage',Path(__file__).with_name('kernel-browser-raster-damage.py'))
raster_damage=importlib.util.module_from_spec(spec);spec.loader.exec_module(raster_damage)

P=c.c_void_p; I=c.c_int; U=c.c_uint; L=c.c_ulong
class Image(c.Structure):
    _fields_=[('width',I),('height',I),('xoffset',I),('format',I),('data',P),('byte_order',I),('bitmap_unit',I),('bitmap_bit_order',I),('bitmap_pad',I),('depth',I),('bytes_per_line',I),('bits_per_pixel',I),('red_mask',L),('green_mask',L),('blue_mask',L),('obdata',P),('funcs',P*6)]
class Shm(c.Structure):
    _fields_=[('shmseg',L),('shmid',I),('shmaddr',P),('readOnly',I)]
class Rectangle(c.Structure):
    _fields_=[('x',c.c_short),('y',c.c_short),('width',c.c_ushort),('height',c.c_ushort)]
class DamageEvent(c.Structure):
    _fields_=[('type',I),('serial',L),('send_event',I),('display',P),('drawable',L),('damage',L),('level',I),('more',I),('timestamp',L),('area',Rectangle),('geometry',Rectangle)]
def bind(lib,name,ret,args):
    f=getattr(lib,name);f.restype=ret;f.argtypes=args;return f
x=c.CDLL(ctypes.util.find_library('X11'));ext=c.CDLL(ctypes.util.find_library('Xext'));damage=c.CDLL(ctypes.util.find_library('Xdamage'));libc=c.CDLL(None);composite=c.CDLL(ctypes.util.find_library('Xcomposite'))
open_display=bind(x,'XOpenDisplay',P,[c.c_char_p]);close_display=bind(x,'XCloseDisplay',I,[P]);root_window=bind(x,'XDefaultRootWindow',L,[P]);visual=bind(x,'XDefaultVisual',P,[P,I]);depth=bind(x,'XDefaultDepth',I,[P,I]);sync=bind(x,'XSync',I,[P,I]);pending=bind(x,'XPending',I,[P]);next_event=bind(x,'XNextEvent',I,[P,P]);fd=bind(x,'XConnectionNumber',I,[P]);free=bind(x,'XFree',I,[P]);destroy_image=bind(x,'XDestroyImage',I,[c.POINTER(Image)]);
query_tree=bind(x,'XQueryTree',I,[P,L,c.POINTER(L),c.POINTER(L),c.POINTER(c.POINTER(L)),c.POINTER(U)]);geometry=bind(x,'XGetGeometry',I,[P,L,c.POINTER(L),c.POINTER(I),c.POINTER(I),c.POINTER(U),c.POINTER(U),c.POINTER(U),c.POINTER(U)]);atom=bind(x,'XInternAtom',L,[P,c.c_char_p,I]);property_=bind(x,'XGetWindowProperty',I,[P,L,L,c.c_long,c.c_long,I,L,c.POINTER(L),c.POINTER(I),c.POINTER(L),c.POINTER(L),c.POINTER(P)])
query_shm=bind(ext,'XShmQueryExtension',I,[P]);create_image=bind(ext,'XShmCreateImage',c.POINTER(Image),[P,P,U,I,P,c.POINTER(Shm),U,U]);attach=bind(ext,'XShmAttach',I,[P,c.POINTER(Shm)]);detach=bind(ext,'XShmDetach',I,[P,c.POINTER(Shm)]);get_image=bind(ext,'XShmGetImage',I,[P,L,c.POINTER(Image),I,I,L]);
query_damage=bind(damage,'XDamageQueryExtension',I,[P,c.POINTER(I),c.POINTER(I)]);create_damage=bind(damage,'XDamageCreate',L,[P,L,I]);subtract=bind(damage,'XDamageSubtract',None,[P,L,L,L]);destroy_damage=bind(damage,'XDamageDestroy',None,[P,L]);
redirect=bind(composite,'XCompositeRedirectWindow',None,[P,L,I]);unredirect=bind(composite,'XCompositeUnredirectWindow',None,[P,L,I]);name_pixmap=bind(composite,'XCompositeNameWindowPixmap',L,[P,L]);free_pixmap=bind(x,'XFreePixmap',I,[P,L]);
shmget=bind(libc,'shmget',I,[I,c.c_size_t,I]);shmat=bind(libc,'shmat',P,[I,P,I]);shmdt=bind(libc,'shmdt',I,[P]);shmctl=bind(libc,'shmctl',I,[I,I,P]);
def pid_of(d,w):
    actual=L();fmt=I();n=L();remaining=L();data=P()
    status=property_(d,w,atom(d,b'_NET_WM_PID',0),0,1,0,6,c.byref(actual),c.byref(fmt),c.byref(n),c.byref(remaining),c.byref(data))
    try:return c.cast(data,c.POINTER(L))[0] if status==0 and fmt.value==32 and n.value==1 and data else None
    finally:
        if data:free(data)
def children(d,w):
    root=L();parent=L();values=c.POINTER(L)();n=U()
    if not query_tree(d,w,c.byref(root),c.byref(parent),c.byref(values),c.byref(n)):return []
    try:return list(values[:n.value])
    finally:
        if values:free(values)
def dims(d,w):
    r=L();xx=I();yy=I();width=U();height=U();border=U();dep=U()
    if not geometry(d,w,c.byref(r),c.byref(xx),c.byref(yy),c.byref(width),c.byref(height),c.byref(border),c.byref(dep)):raise ValueError('window unavailable')
    return width.value,height.value

d=None;image=None;shm=Shm(shmid=-1);attached=False;dam=0;pixmap=0;window=0
pool=[];free_slots=set();leased={}
stage='start';config={}
try:
    config=json.loads(sys.stdin.buffer.readline());owner=config['pid'];width=config['width'];height=config['height']
    if not isinstance(owner,int) or owner<=1 or not raster_damage.capture_geometry_allowed(width,height):raise ValueError('admission')
    if config.get('pool'):
        root=Path(config['pool'])
        if not root.is_absolute() or root.is_symlink() or root.stat().st_uid!=os.getuid() or root.stat().st_mode & 0o077:raise ValueError('pool owner')
        for slot in range(3):
            file=os.open(root/str(slot),os.O_CREAT|os.O_EXCL|os.O_RDWR|os.O_NOFOLLOW,0o600);os.ftruncate(file,width*height*4)
            pool.append(mmap.mmap(file,width*height*4));os.close(file);free_slots.add(slot)
    d=open_display(os.environ['DISPLAY'].encode())
    if not d or not query_shm(d):raise ValueError('XShm unavailable')
    event_base=I();error_base=I()
    if not query_damage(d,c.byref(event_base),c.byref(error_base)):raise ValueError('XDamage unavailable')
    stage='window'
    desktop=config.get('desktop') is True
    if desktop:
        spec=importlib.util.spec_from_file_location('native_accessibility',Path(__file__).with_name('native-accessibility.py'))
        accessibility=importlib.util.module_from_spec(spec);spec.loader.exec_module(accessibility)
    windows=[w for w in children(d,root_window(d)) if pid_of(d,w)==owner and dims(d,w)[0]==width and dims(d,w)[1]>=height]
    if desktop:windows=[root_window(d)]
    if len(windows)!=1:raise ValueError('owned browser window ambiguous')
    window=windows[0];ww,hh=dims(d,window);offset=hh-height
    # Bottom-aligned viewport is independently attested against CDP before use.
    if offset>400:raise ValueError('viewport crop bound')
    if desktop:
        if dims(d,window)!=(width,height):raise ValueError('owned desktop geometry')
        pixmap=window
    else:
        redirect(d,window,0);sync(d,0);pixmap=name_pixmap(d,window);sync(d,0)
    if not pixmap:raise ValueError('window backing unavailable')
    stage='image'
    image=create_image(d,visual(d,0),depth(d,0),2,None,c.byref(shm),width,height)
    if not image or image.contents.bits_per_pixel!=32 or image.contents.byte_order!=0 or (image.contents.red_mask,image.contents.green_mask,image.contents.blue_mask)!=(0xff0000,0xff00,0xff):raise ValueError('pixel format')
    size=image.contents.bytes_per_line*height
    if size!=width*height*4:raise ValueError('stride bound')
    stage='shm'
    shm.shmid=shmget(0,size,0o1000|0o600)
    if shm.shmid<0:raise ValueError('shm allocation')
    shm.shmaddr=shmat(shm.shmid,None,0)
    if shm.shmaddr==c.c_void_p(-1).value:shm.shmaddr=None;raise ValueError('shm attach')
    image.contents.data=shm.shmaddr
    if not attach(d,c.byref(shm)):raise ValueError('server attach')
    attached=True;sync(d,0);shmctl(shm.shmid,0,None) # IPC_RMID now: OS reclaims on crash too.
    stage='damage'
    dam=create_damage(d,window,0) # RawRectangles; retain damage union, coalesce to60Hz.
    event=(L*24)();dirty=True;area=[0,0,width,height];last=0;signature=None;serial=0;previous=None;damage_ready_ms=time.time()*1000
    fingerprint=raster_damage.RasterFingerprint(width,height);control=b'';urgent_until=0;wake_ms=None;refresh=False
    while True:
        if not pending(d):select.select([fd(d),sys.stdin.fileno()],[],[],raster_damage.capture_wait(last,dirty,time.monotonic(),urgent_until))
        if select.select([sys.stdin.fileno()],[],[],0)[0]:
            data=os.read(sys.stdin.fileno(),4096)
            if not data:break
            control+=data
            if len(control)>(1<<20 if desktop else 8192):raise ValueError('pool control bound')
            while b'\n' in control:
                line,control=control.split(b'\n',1)
                release=json.loads(line)
                if release == {'wake':True}:
                    urgent_until=time.monotonic()+.1;wake_ms=time.time()*1000
                    continue
                if desktop and set(release)-{'values'}=={'refresh','processes','browser_processes','browser_protection','protection_serial'} and release['refresh'] is True:
                    # Owner 2026-10-09: Vault values are masked best effort by text box.
                    values=release.get('values',[])
                    if not isinstance(values,list) or len(values)>256 or not all(isinstance(v,str) and 0<len(v)<=4096 for v in values):raise ValueError('desktop values')
                    config['values']=values
                    for field in ('processes','browser_processes'):
                        if not isinstance(release[field],list) or len(release[field])>256:raise ValueError('desktop process scope')
                        config[field]=release[field]
                    # MP-08/MP-11: CDP regions the kernel measured stable and
                    # presented; null withholds every kernel-browser window.
                    if release['browser_protection'] is not None and not isinstance(release['browser_protection'],dict) or type(release['protection_serial']) is not int:raise ValueError('desktop protection')
                    config['browser_protection']=release['browser_protection'];config['protection_serial']=release['protection_serial']
                    release={'refresh':True}
                if release == {'refresh':True}:
                    # MP-11: trusted protection changes can be paint-free.
                    # One complete readback per coalesced refresh, no polling.
                    dirty=True;refresh=True;area=[0,0,width,height]
                    urgent_until=time.monotonic()+.1;wake_ms=time.time()*1000
                    continue
                slot=release.get('release')
                if type(slot) is not int or leased.get(slot)!=release.get('serial'):raise ValueError('pool release')
                del leased[slot];free_slots.add(slot)
        while pending(d):
            next_event(d,c.byref(event))
            if c.cast(event,c.POINTER(I))[0]==event_base.value:
                rect=c.cast(event,c.POINTER(DamageEvent)).contents.area
                x0=max(0,rect.x);y0=max(0,rect.y-offset);x1=min(width,rect.x+rect.width);y1=min(height,rect.y+rect.height-offset)
                if x1>x0 and y1>y0:
                    if not dirty:damage_ready_ms=time.time()*1000
                    area=[min(area[0],x0),min(area[1],y0),max(area[2],x1),max(area[3],y1)] if dirty else [x0,y0,x1,y1];dirty=True
        if not raster_damage.capture_due(last,dirty,time.monotonic(),urgent_until):continue
        if pool and not free_slots:
            select.select([sys.stdin.fileno()],[],[],.016);continue
        # MP-08/MP-10: pace capture starts, not completion of readback/hash.
        # Adding their cost to16ms misses the next60Hz damage notification.
        input_wake_ms=wake_ms if time.monotonic()<urgent_until else None
        last=time.monotonic();urgent_until=0;wake_ms=None;at=time.time()*1000
        if (not desktop and pid_of(d,window)!=owner) or dims(d,window)!=(ww,hh):raise ValueError('window fence')
        protection_serial=config.get('protection_serial',0)
        if desktop:before=accessibility.snapshot(config.get('processes',[]),config.get('browser_processes',[]),config.get('browser_protection'),config.get('values',[]))
        stage='get_image'
        get_image_ms=time.time()*1000
        if not get_image(d,pixmap,image,0,offset,0xffffffff):raise ValueError('readback')
        image_ready_ms=time.time()*1000
        # XShm avoids Xlib pixel IPC. One bounded copy crosses the helper pipe.
        raw=c.string_at(shm.shmaddr,size);readback_ms=time.time()*1000
        protected_regions=[]
        if desktop:
            after=accessibility.snapshot(config.get('processes',[]),config.get('browser_processes',[]),config.get('browser_protection'),config.get('values',[]))
            # MP-08/MP-11: native password dots stay visible; saved plaintext
            # values use the same local AT-SPI masks as on-demand capture.
            if config.get('mask') or before!=after or not before.get('available') or not before.get('complete'):
                protected_regions=[[0,0,width,height]]
            else:protected_regions=before.get('masks',before.get('uncovered',[]))
            protected=bytearray(raw)
            for rx,ry,rw,rh in protected_regions:
                if any(type(v) is not int for v in [rx,ry,rw,rh]) or rx<0 or ry<0 or rw<1 or rh<1 or rx+rw>width or ry+rh>height:
                    protected_regions=[[0,0,width,height]];protected=bytearray(size);break
                for row in range(ry,ry+rh):protected[(row*width+rx)*4:(row*width+rx+rw)*4]=bytes(rw*4)
            raw=bytes(protected)
        sig=fingerprint.update(raw);fingerprint_ms=time.time()*1000;dirty=False
        if not fingerprint.changed_bands and not refresh:continue
        refresh=False
        signature=sig;serial+=1
        # Full native readback/fingerprint remains authoritative. Avoid moving
        # 16MiB through the private pipe for a small changed rectangle. Receiver
        # rebuilds each serial before coalescing samples; missed bases fail shut.
        patch,payload=raster_damage.raster_payload(previous,raw,width,height,area,fingerprint)
        # Both sparse delivery and native exact tiles use the readback delta.
        area=patch or [0,0,width,height];previous=raw;damage_ms=time.time()*1000
        slot=None
        if pool:
            slot=min(free_slots);free_slots.remove(slot);leased[slot]=serial
            pool[slot][:]=raw;payload=b"\0";patch=None
        header=json.dumps(dict(protected_regions=protected_regions,protection_serial=protection_serial,input_wake_ms=input_wake_ms,slot=slot,width=width,height=height,length=len(payload),serial=serial,base_serial=serial-1,patch=patch,signature=sig,captured_ms=at,capture_ms=time.time()*1000-at,readback_ms=readback_ms,fingerprint_ms=fingerprint_ms,damage_ms=damage_ms,damage=area,window_height=hh,offset=offset,damage_ready_ms=damage_ready_ms,get_image_ms=get_image_ms,image_ready_ms=image_ready_ms)).encode()
        sys.stdout.buffer.write(struct.pack('!I',len(header))+header+payload);sys.stdout.buffer.flush()
except Exception:
    sys.stderr.write('MD-DISPLAY: native stage '+stage+'\n');sys.exit(1)
finally:
    if d and pixmap and not config.get('desktop'):free_pixmap(d,pixmap);unredirect(d,window,0)
    if d and dam:destroy_damage(d,dam)
    if d and attached:detach(d,c.byref(shm));sync(d,0)
    if image:image.contents.data=None;destroy_image(image)
    if shm.shmaddr:shmdt(shm.shmaddr)
    if shm.shmid>=0:shmctl(shm.shmid,0,None)
    if d:close_display(d)
    for mapping in pool:mapping.close()
