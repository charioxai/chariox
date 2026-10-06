"""MP-08/MP-10/MP-11: original bounded row encoders over admitted raster bytes.
Encoder choice is pluggable: libx264 (GPL) or libopenh264 (BSD), when available.
No capture, root/window access, credentials or external authority in this module.
"""
import base64
import ctypes
import ctypes.util
from fractions import Fraction
import os
import re
import av

_xxh=ctypes.CDLL(ctypes.util.find_library('xxhash') or 'libxxhash.so.0').XXH3_64bits
_xxh.argtypes=[ctypes.c_void_p,ctypes.c_size_t];_xxh.restype=ctypes.c_uint64

def row_geometry(height):
    if type(height) is not int or height<16 or height%2:raise ValueError('stripe height')
    edges=[2*((height//2*i)//8) for i in range(9)]
    return [(edges[i],edges[i+1]-edges[i]) for i in range(8)]

class StripeEncoder:
    def __init__(self):
        self.rows={};self.config=None
        self.backend=os.environ.get('CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER','libx264')
        if self.backend not in ('libx264','libopenh264','libvpx'):raise ValueError('encoder selection')

    def encode(self,pixels,width,height,bitrate,reset=False,selected="avc1.420033"):
        if type(width) is not int or width<16 or width>2560 or width%2 or height>1600 or len(pixels)!=width*height*4:raise ValueError('stripe geometry')
        backend="libvpx" if selected=="vp8" else self.backend
        if selected not in ("vp8","avc1.420033") or (selected=="avc1.420033" and backend=="libvpx"):raise ValueError("stripe codec selection")
        self.effective_backend=backend
        config=(width,height,bitrate,backend)
        if config!=self.config:self.rows={};self.config=config;reset=True
        resets=set(range(8)) if reset is True else set(reset or [])
        if any(type(i) is not int or i<0 or i>=8 for i in resets):raise ValueError('stripe reset')
        output=[]
        for row,(y,h) in enumerate(row_geometry(height)):
            data=bytes(pixels[y*width*4:(y+h)*width*4])
            signature=_xxh(ctypes.cast(ctypes.c_char_p(data),ctypes.c_void_p),len(data))
            old=self.rows.get(row)
            # Fast hash is only a prefilter. Equal hash compares exact admitted
            # bytes, so a collision cannot omit a changed row.
            if old and row not in resets and old['hash']==signature and old['pixels']==data:continue
            if old is None or row in resets:
                codec=av.CodecContext.create(backend,'w');codec.width=width;codec.height=h
                codec.pix_fmt='yuv420p';codec.time_base=Fraction(1,60);codec.framerate=Fraction(60,1)
                codec.bit_rate=max(16000,int(bitrate*.45*h/height));codec.thread_count=1
                if backend=='libx264':
                    codec.options={'preset':'ultrafast','tune':'zerolatency','profile':'baseline','level':'5.1','crf':'23','g':'120','bf':'0','forced-idr':'1','x264-params':'sync-lookahead=0:repeat-headers=1:annexb=1:rc-lookahead=0'}
                elif backend=='libopenh264':codec.options={'profile':'constrained_baseline','allow_skip_frames':'0','rc_mode':'bitrate','max_nal_size':'0'}
                else:codec.options={'deadline':'realtime','cpu-used':'8','lag-in-frames':'0','g':'120','error-resilient':'1'}
                old={'codec':codec,'sequence':0};self.rows[row]=old
            codec=old['codec'];frame=av.VideoFrame(width,h,'bgr0');frame.planes[0].update(data)
            frame=frame.reformat(format='yuv420p');frame.time_base=codec.time_base;frame.pts=old['sequence']
            frame.pict_type=av.video.frame.PictureType.I if old['sequence']==0 else av.video.frame.PictureType.NONE
            packets=list(codec.encode(frame))
            if len(packets)!=1:raise ValueError('stripe packet count')
            packet=bytes(packets[0]);key=packets[0].is_keyframe if selected=='vp8' else any(nal and nal[0]&31==5 for nal in re.split(b'\x00\x00\x01',packet))
            previous=old['sequence'];old.update(sequence=previous+1,hash=signature,pixels=data)
            output.append(dict(row=row,y=y,height=h,codec=selected,key=key,sequence=previous+1,reference_sequence=None if key else previous,data_base64=base64.b64encode(packet).decode()))
        if sum(len(row['data_base64']) for row in output)>1024*1024:raise ValueError('stripe payload bound')
        return output
