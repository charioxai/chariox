"""MP-08/MP-10/MP-11: original bounded row encoders over admitted raster bytes.
Encoder choice is pluggable: libx264 (GPL) or libopenh264 (BSD), when available.
No capture, root/window access, credentials or external authority in this module.
"""
import base64
from concurrent.futures import ThreadPoolExecutor
import ctypes
import ctypes.util
from fractions import Fraction
import os
import re
import time
import av
import importlib.util
from pathlib import Path
_spec=importlib.util.spec_from_file_location('codec_protection',Path(__file__).with_name('kernel-browser-codec-protection.py'))
_protection=importlib.util.module_from_spec(_spec);_spec.loader.exec_module(_protection)

_xxh=ctypes.CDLL(ctypes.util.find_library('xxhash') or 'libxxhash.so.0').XXH3_64bits
_xxh.argtypes=[ctypes.c_void_p,ctypes.c_size_t];_xxh.restype=ctypes.c_uint64
_memcmp=ctypes.CDLL(None).memcmp
_memcmp.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_size_t];_memcmp.restype=ctypes.c_int

def row_geometry(height):
    if type(height) is not int or height<16 or height%2:raise ValueError('stripe height')
    edges=[2*((height//2*i)//8) for i in range(9)]
    return [(edges[i],edges[i+1]-edges[i]) for i in range(8)]

class BgrConverter:
    """MP-08/MP-10: write SIMD I420 directly into AV planes; no BGR AV copy.
    Optional BSD libyuv stays in the helper. Missing auto discovery uses PyAV.
    An explicit library failure is surfaced rather than changing that choice.
    """
    def __init__(self,library=None):
        explicit=library or os.environ.get('CHARIOX_BROWSER_DISPLAY_LIBYUV')
        name=None if library is False or explicit=='off' else explicit or ctypes.util.find_library('yuv')
        self.convert_native=None
        if name:
            try:
                self.library=ctypes.CDLL(name)
                self.convert_native=self.library.ARGBToI420
                self.convert_native.argtypes=[ctypes.c_void_p,ctypes.c_int]*4+[ctypes.c_int]*2
                self.convert_native.restype=ctypes.c_int
            except OSError:
                if explicit:raise
        self.reformatter=av.video.reformatter.VideoReformatter()

    def convert(self,data,width,height):
        if width<16 or width>2560 or height<2 or height>1600 or width%2 or height%2 or len(data)!=width*height*4:
            raise ValueError('BGR conversion geometry')
        if not self.convert_native:
            frame=av.VideoFrame(width,height,'bgr0');plane=frame.planes[0]
            if plane.line_size!=width*4:
                padded=bytearray(plane.buffer_size)
                for row in range(height):padded[row*plane.line_size:row*plane.line_size+width*4]=data[row*width*4:(row+1)*width*4]
                plane.update(padded)
            else:plane.update(data)
            return self.reformatter.reformat(frame,format='yuv420p')
        frame=av.VideoFrame(width,height,'yuv420p')
        args=[ctypes.cast(ctypes.c_char_p(data),ctypes.c_void_p),width*4]
        for plane in frame.planes:args.extend([plane.buffer_ptr,plane.line_size])
        if self.convert_native(*args,width,height)!=0:raise ValueError('BGR conversion failed')
        return frame

class StripeEncoder:
    def __init__(self):
        self.rows={};self.config=None
        self.workers=int(os.environ.get('CHARIOX_BROWSER_DISPLAY_STRIPE_WORKERS','1'))
        if self.workers not in (1,2,4):raise ValueError('stripe workers')
        self.pool=ThreadPoolExecutor(max_workers=self.workers) if self.workers>1 else None
        self.backend=os.environ.get('CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER','libx264')
        if self.backend not in ('libx264','libopenh264','libvpx'):raise ValueError('encoder selection')

    def encode(self,pixels,width,height,bitrate,reset=False,selected="avc1.420033",regions=None):
        # MP-08/MP-10: private, opt-in stage clocks; no raster/region data.
        self.timings=[]
        tracing=os.environ.get('CHARIOX_BROWSER_DISPLAY_TIMING')=='1'
        def stamp():return time.time_ns()/1000000 if tracing else 0
        def timing(stage,start):
            if tracing:self.timings.append([stage,start,stamp()])
        self.protection_dropped=False
        if type(width) is not int or width<16 or width>2560 or width%2 or height>1600 or len(pixels)!=width*height*4:raise ValueError('stripe geometry')
        backend="libvpx" if selected=="vp8" else self.backend
        if selected not in ("vp8","avc1.420033") or (selected=="avc1.420033" and backend=="libvpx"):raise ValueError("stripe codec selection")
        self.effective_backend=backend
        regions=[] if regions is None else regions
        at=stamp();_protection.black_input(pixels,width,height,regions);timing('codec_input_guard',at)
        protected_bounds=_protection.bounds(regions,width,height)
        # MP-08/MP-10: retain the admitted buffer through synchronous workers,
        # but hash/compare its rows in place. Copy only rows entering a codec.
        # No ctypes or memoryview export may survive the exchange's mmap close.
        if isinstance(pixels,bytes):address=ctypes.cast(ctypes.c_char_p(pixels),ctypes.c_void_p).value
        else:
            try:address=ctypes.addressof(ctypes.c_char.from_buffer(pixels))
            except (TypeError,BufferError):
                pixels=bytes(pixels);address=ctypes.cast(ctypes.c_char_p(pixels),ctypes.c_void_p).value
        config=(width,height,bitrate,backend,repr(regions))
        if config!=self.config:self.rows={};self.config=config;reset=True
        resets=set(range(8)) if reset is True else set(reset or [])
        if any(type(i) is not int or i<0 or i>=8 for i in resets):raise ValueError('stripe reset')
        output=[]
        def encode_row(item):
            row,(y,h)=item
            at=stamp()
            start=y*width*4;length=h*width*4
            signature=_xxh(address+start,length)
            timing('codec_row_copy_hash',at)
            old=self.rows.get(row)
            # Fast hash is only a prefilter. Equal hash compares exact admitted
            # bytes, so a collision cannot omit a changed row.
            if old and row not in resets and old['hash']==signature and _memcmp(ctypes.cast(ctypes.c_char_p(old['pixels']),ctypes.c_void_p),address+start,length)==0:return None
            data=bytes(memoryview(pixels)[start:start+length])
            if old is None or row in resets:
                at=stamp()
                if backend=='libopenh264' and os.environ.get('CHARIOX_BROWSER_DISPLAY_OPENH264_ADAPTER'):
                    spec=importlib.util.spec_from_file_location('native_openh264',Path(__file__).with_name('kernel-browser-openh264.py'));module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
                    codec=module.NativeOpenH264(width,h,max(16000,int(bitrate*.45*h/height)))
                else:codec=av.CodecContext.create(backend,'w')
                codec.width=width;codec.height=h
                codec.pix_fmt='yuv420p';codec.time_base=Fraction(1,60);codec.framerate=Fraction(60,1)
                # MP-08/MP-10: Selkies/pixelflux CBR at the row's share of the
                # negotiated rate, 1.5-frame VBV, infinite GOP (native codec.c).
                codec.bit_rate=max(16000,int(bitrate*h/height));codec.thread_count=1
                if backend=='libx264':
                    codec.options={'preset':'ultrafast','tune':'zerolatency','profile':'baseline','level':'5.1','g':'-1','bf':'0','forced-idr':'1','x264-params':f'keyint=infinite:scenecut=0:sync-lookahead=0:repeat-headers=1:annexb=1:rc-lookahead=0:bitrate={max(16,codec.bit_rate//1000)}:vbv-maxrate={max(16,codec.bit_rate//1000)}:vbv-bufsize={max(16,codec.bit_rate*3//120000)}:filler=0'}
                elif backend=='libopenh264':codec.options={'profile':'constrained_baseline','allow_skip_frames':'0','rc_mode':'bitrate','max_nal_size':'0'}
                else:codec.options={'deadline':'realtime','cpu-used':'8','lag-in-frames':'0','g':'120','error-resilient':'1','bufsize':str(max(16000,codec.bit_rate//20)),'maxrate':str(codec.bit_rate),'minrate':'0','undershoot-pct':'95','overshoot-pct':'5','qmin':'4','qmax':'48','rc_init_occupancy':str(max(16000,codec.bit_rate//20)),'max-intra-rate':'200'}
                old={'codec':codec,'sequence':0,'converter':BgrConverter(),
                     'guard':_protection.DecodedMaskGuard(selected) if any(left<right and top<y+h and bottom>y for left,top,right,bottom in protected_bounds) else None};self.rows[row]=old
                timing('codec_row_init',at)
            at=stamp();codec=old['codec'];frame=old['converter'].convert(data,width,h);timing('codec_convert',at)
            frame.time_base=codec.time_base;frame.pts=old['sequence']
            frame.pict_type=av.video.frame.PictureType.I if old['sequence']==0 else av.video.frame.PictureType.NONE
            at=stamp();packets=list(codec.encode(frame));timing('codec_encode',at)
            if len(packets)!=1:raise ValueError('stripe packet count')
            packet=bytes(packets[0]);key=packets[0].is_keyframe if selected=='vp8' else any(nal and nal[0]&31==5 for nal in re.split(b'\x00\x00\x01',packet))
            if old['guard']:
                at=stamp();safe=old['guard'].safe(packet,regions,width,h,y=y);timing('codec_output_guard',at)
                if not safe:return False
            previous=old['sequence'];old.update(sequence=previous+1,hash=signature,pixels=data)
            at=stamp();result=dict(row=row,y=y,height=h,codec=selected,key=key,sequence=previous+1,reference_sequence=None if key else previous,data_base64=base64.b64encode(packet).decode());timing('codec_base64',at)
            return result
        # MP-10: one bounded pool, one codec thread per independent row.
        # Unchanged rows return before codec conversion or rate control.
        work=enumerate(row_geometry(height))
        output=[row for row in (self.pool.map(encode_row,work) if self.pool and backend=='libx264' else map(encode_row,work)) if row is not None]
        if any(row is False for row in output):
            # Drop before packetization and retire advanced codec references.
            # The next source damage starts independent row chains.
            self.rows={}
            self.protection_dropped=True
            return []
        if sum(len(row['data_base64']) for row in output)>1024*1024:raise ValueError('stripe payload bound')
        return output

    def close(self):
        if self.pool:self.pool.shutdown(wait=True);self.pool=None
