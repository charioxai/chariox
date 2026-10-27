"""MP-08/MP-10/MP-11: bounded Cisco screen encoder in the separate helper."""
import ctypes as c
from fractions import Fraction
import os
import av

class Packet:
    def __init__(self,data):self.data=data
    def __bytes__(self):return self.data

class NativeOpenH264:
    def __init__(self,width,height,bitrate):
        self.library=c.CDLL(os.environ['CHARIOX_BROWSER_DISPLAY_OPENH264_ADAPTER'])
        create=self.library.chariox_h264_create;create.argtypes=[c.c_int]*3;create.restype=c.c_void_p
        close=self.library.chariox_h264_close;close.argtypes=[c.c_void_p];close.restype=None
        encode=self.library.chariox_h264_encode
        encode.argtypes=[c.c_void_p]*4+[c.c_int]*3+[c.c_int64,c.c_int,c.c_void_p,c.c_int];encode.restype=c.c_int
        if self.library.chariox_h264_version()!=20600:raise ValueError('OpenH264 version pin')
        self.width=width;self.height=height;self.time_base=Fraction(1,60)
        self.handle=create(width,height,bitrate)
        if not self.handle:raise ValueError('OpenH264 screen admission')
        self.output=c.create_string_buffer(1024*1024)
    def encode(self,frame):
        if (frame.width,frame.height)!=(self.width,self.height) or frame.format.name!='yuv420p':raise ValueError('OpenH264 frame geometry')
        planes=frame.planes
        size=self.library.chariox_h264_encode(self.handle,*[p.buffer_ptr for p in planes],*[p.line_size for p in planes],int(frame.pts*1000/60),int(frame.pict_type==av.video.frame.PictureType.I),self.output,len(self.output))
        if size<1 or size>len(self.output):raise ValueError('OpenH264 output bound')
        return [Packet(c.string_at(self.output,size))]
    def close(self):
        if getattr(self,'handle',None):self.library.chariox_h264_close(self.handle);self.handle=None
    def __del__(self):self.close()
