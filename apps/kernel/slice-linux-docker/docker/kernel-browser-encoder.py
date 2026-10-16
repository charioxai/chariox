"""MD-DISPLAY-04: bounded persistent VP9 stream of protected PNG frames only."""
import base64
import io
import re
import hashlib
import json
import sys
import os
import shutil
import mmap
import stat
import subprocess
import threading
import uuid
import time
from pathlib import Path
from fractions import Fraction
import av
import importlib.util
_spec=importlib.util.spec_from_file_location('codec_protection',Path(__file__).with_name('kernel-browser-codec-protection.py'))
protection=importlib.util.module_from_spec(_spec);_spec.loader.exec_module(protection)


def h264_idr(packet):
    # Recovery-point SEI / periodic intra-refresh is not an independent IDR.
    # The presenter resets its decoder only for actual IDR access units.
    return any(nal and nal[0] & 31 == 5 for nal in re.split(b'\x00\x00\x01',packet))

class VaapiEncoder:
    """MD-DISPLAY-02/04: owned persistent FFmpeg VAAPI with framed NUT output.
    It consumes admitted raw pixels only; it never opens a capture source.
    Read/write failures/timeouts kill only its validated child and fall back.
    """
    def __init__(self):
        self.child = None
        self.container = None
        self.configuration = None

    @staticmethod
    def stop_child(child):
        if child is not None:
            if not isinstance(child.pid,int) or child.pid <= 1:
                raise ValueError('MD-DISPLAY: unsafe encoder child PID')
            if child.poll() is None:child.kill()

    def abort(self):
        self.stop_child(self.child)

    def encode(self, frame, bitrate, reset=False):
        config = (frame.width,frame.height,bitrate)
        if self.child is None or reset or config != self.configuration:
            self.close()
            device = next(p for p in sorted(Path('/dev/dri').glob('renderD*')) if os.access(p,os.R_OK|os.W_OK))
            command = ['ffmpeg','-v','error','-nostdin','-init_hw_device',f'vaapi=chariox:{device}',
                '-filter_hw_device','chariox','-f','rawvideo','-pixel_format','bgr0',
                '-video_size',f'{frame.width}x{frame.height}','-framerate','60',
                '-probesize','32','-analyzeduration','0','-i','pipe:0','-an',
                '-vf','format=nv12,hwupload','-c:v','h264_vaapi','-profile:v','constrained_baseline',
                '-level:v','5.1','-bf','0','-g','120','-b:v',str(int(bitrate*.45)),
                '-maxrate',str(int(bitrate*.45)),'-bufsize',str(max(32000,int(bitrate*.05))),
                '-flags','+low_delay','-bsf:v','h264_mp4toannexb','-f','nut','-flush_packets','1','pipe:1']
            self.child = subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
            self.configuration = config
        child = self.child
        # Writer/readback must progress independently for a native16MiB frame.
        # NUT preserves packet sizes (AnnexB pipe reads do not frame access units).
        errors=[]
        def write():
            try:
                pixels = bytes(frame.reformat(format='bgr0').planes[0])
                if len(pixels)!=frame.width*frame.height*4:raise ValueError('VAAPI raw stride')
                child.stdin.write(pixels);child.stdin.flush()
            except Exception as error:errors.append(error);self.stop_child(child)
        timer = threading.Timer(5,lambda:self.stop_child(child))
        writer = threading.Thread(target=write,daemon=True)
        timer.start();writer.start()
        try:
            if self.container is None:
                self.container=av.open(child.stdout,format='nut',mode='r',buffer_size=1,options={'probesize':'32','analyzeduration':'0'})
                self.packets=self.container.demux(video=0)
            packet=next(self.packets)
            writer.join(timeout=1)
            if writer.is_alive() or errors or packet.size<1 or packet.size>3*1024*1024:raise ValueError('VAAPI packet bound')
            return packet
        except Exception:
            self.abort();writer.join(timeout=1)
            raise
        finally:timer.cancel()

    def close(self):
        self.abort()
        if self.container is not None:self.container.close();self.container=None
        if self.child is not None:
            child=self.child;self.child=None
            child.wait(timeout=2);child.stdin.close();child.stdout.close()
        self.configuration=None

def main():
    stripes = None
    codec = None
    configuration = None
    sequence = 0
    hardware = None
    hardware_disabled = os.environ.get('CHARIOX_BROWSER_DISPLAY_SOFTWARE') == '1'
    mask_guard = None
    while True:
        line=sys.stdin.buffer.readline()
        if not line:break
        mapping = None
        try:
            request = json.loads(line)
            regions=request.get('protected_regions',[])
            if 'raw' in request:
                raw=request['raw'];w=raw['width'];h=raw['height'];size=w*h*4
                if not (1<=w<=2560 and 1<=h<=1600) or raw['format']!='bgr0' or raw['length']!=size:raise ValueError('raw bound')
                if raw.get('shared'):
                    shared=raw['shared']
                    if shared.get('length')!=size:raise ValueError('shared size')
                    file=os.open(shared['path'],os.O_RDONLY|os.O_NOFOLLOW)
                    try:
                        info=os.fstat(file)
                        if not stat.S_ISREG(info.st_mode) or info.st_uid!=os.getuid() or info.st_mode & 0o077 or info.st_size!=size:raise ValueError('shared owner')
                        # MP-08/MP-10: private COW permits a temporary ctypes
                        # address for exact row comparisons; no writer touches
                        # it, and the capture/snapshot file remains immutable.
                        mapping=mmap.mmap(file,size,access=mmap.ACCESS_COPY)
                    finally:os.close(file)
                    pixels=mapping
                else:pixels=sys.stdin.buffer.read(size)
                if len(pixels)!=size:raise ValueError('raw truncated')
                if raw.get('mask_shared'):
                    if mapping is None or not regions:raise ValueError('private mask lease unavailable')
                    protection.mask_private(pixels,w,h,regions)
                if request.get('operation')!='stripes':
                    frame=av.VideoFrame(w,h,'bgr0')
                    frame.planes[0].update(pixels)

            else:
                png = base64.b64decode(request['png'], validate=True)
                if len(png) > 4 * 1024 * 1024:raise ValueError('frame bound')
                with av.open(io.BytesIO(png)) as source:frame = next(source.decode(video=0))
            if request.get('operation')!='stripes' and (frame.width > 2560 or frame.height > 1600):
                raise ValueError('geometry bound')
            if request.get('operation')=='stripes':
                import importlib.util
                if stripes is None:
                    spec=importlib.util.spec_from_file_location('stripes',Path(__file__).with_name('kernel-browser-stripes.py'))
                    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);stripes=module.StripeEncoder()
                if 'raw' not in request:raise ValueError('stripes require admitted raster')
                rows=stripes.encode(pixels,w,h,request['bitrate'],request.get('reset',False),request.get('codec','avc1.420033'),regions)
                # MP-08/MP-10: packetize all rows once in the native helper. Node
                # receives headers only; the kernel consumes the private file.
                reply={'stripes':rows,'backend':stripes.effective_backend,'workers':stripes.workers if stripes.effective_backend=='libx264' else 1,'converter':'libyuv' if all(row['converter'].convert_native for row in stripes.rows.values()) else 'swscale'}
                if stripes.protection_dropped:reply['dropped']=True
                at=time.time_ns()/1000000
                root=os.environ.get('CHARIOX_BROWSER_DISPLAY_PACKET_ROOT')
                if root and rows:
                    directory=os.open(root,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
                    try:
                        info=os.fstat(directory)
                        if info.st_uid!=os.getuid() or info.st_mode & 0o077:raise ValueError('packet root owner')
                        # MP-08/MP-10/MP-11: protocol 466 packet: u32 header length,
                        # JSON segment headers with lengths, then raw segments.
                        segments=[base64.b64decode(row['data_base64']) for row in rows]
                        headers=[{**{k:v for k,v in row.items() if k!='data_base64'},'length':len(data)} for row,data in zip(rows,segments)]
                        header=json.dumps(headers,separators=(',',':')).encode()
                        name=uuid.uuid4().hex+'.json';payload=len(header).to_bytes(4,'big')+header+b''.join(segments)
                        if len(payload)>1024*1024:raise ValueError('packet bound')
                        file=os.open(name,os.O_CREAT|os.O_EXCL|os.O_WRONLY|os.O_NOFOLLOW,0o600,dir_fd=directory)
                        try:
                            with os.fdopen(file,'wb') as output:output.write(payload)
                        except Exception:
                            os.unlink(name,dir_fd=directory);raise
                        reply['packet']={'name':name,'length':len(payload)}
                        reply['stripes']=headers
                    finally:os.close(directory)
                if os.environ.get('CHARIOX_BROWSER_DISPLAY_TIMING')=='1':reply['timings']=[*stripes.timings,['codec_packetize',at,time.time_ns()/1000000]]
                print(json.dumps(reply,separators=(',',':')),flush=True);continue
            if request.get('operation')=='fingerprint':
                signature=hashlib.sha256(frame.format.name.encode())
                for plane in frame.planes:
                    bpp=3 if frame.format.name in ('rgb24','bgr24') else 4 if frame.format.name in ('rgba','bgra') else 1
                    pixels=memoryview(plane);stride=plane.width*bpp
                    if stride==plane.line_size:signature.update(pixels[:stride*plane.height])
                    else:
                        for row in range(plane.height):signature.update(pixels[row*plane.line_size:row*plane.line_size+stride])
                print(json.dumps({'signature':signature.hexdigest(),'width':frame.width,'height':frame.height}),flush=True)
                continue
            if request.get('operation') is not None:raise ValueError('operation bound')
            source_width,source_height=frame.width,frame.height
            if regions:
                original=frame.reformat(format='bgr0');plane=original.planes[0]
                source=bytes(plane)
                source=b''.join(source[y*plane.line_size:y*plane.line_size+frame.width*4] for y in range(frame.height))
                protection.black_input(source,frame.width,frame.height,regions)
            selected=request.get('codec','vp09.00.10.08')
            if selected not in ('vp8','vp09.00.10.08','vp09.00.40.08','vp09.00.50.08','avc1.420033'):raise ValueError('codec admission')
            if selected.startswith('avc1') and not regions and not hardware_disabled and shutil.which('ffmpeg') and any(Path('/dev/dri').glob('renderD*')):
                try:
                    if hardware is None:hardware = VaapiEncoder()
                    packet = hardware.encode(frame, request['bitrate'], request.get('reset',False))
                    print(json.dumps({'data_base64':base64.b64encode(bytes(packet)).decode(),'key':h264_idr(bytes(packet)),'backend':'vaapi'}),flush=True)
                    continue
                except Exception:
                    if hardware is not None:hardware.close()
                    hardware = None
                    hardware_disabled = True
                    codec = None  # backend switch must be an independent frame
                    request['reset'] = True
            # Software motion at <=16Mbps trades transient resolution for
            # cadence. Hardware retains native DPR; protected exact PNG/tiles
            # never pass through this branch. Bounded experimental clients accept
            # CSS-sized or1080p-to720p video and upscale into the native canvas.
            if request.get('raw',{}).get('motion') is True and (frame.width,frame.height)==(2560,1600) and request['bitrate']<=16000000:
                frame=frame.reformat(width=1280,height=800,format='yuv420p')
            elif request.get('raw',{}).get('motion') is True and (frame.width,frame.height)==(1920,1080) and request['bitrate']<=16000000:
                frame=frame.reformat(width=1280,height=720,format='yuv420p',interpolation='FAST_BILINEAR')
            software_h264=os.environ.get('CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER','libx264')
            if selected.startswith('avc1') and software_h264 not in ('libx264','libopenh264'):raise ValueError('software H264 selection')
            config = (frame.width, frame.height, request['bitrate'], selected,software_h264,repr(regions))
            if codec is None or config != configuration:
                if selected.startswith('avc1') and software_h264=='libopenh264' and os.environ.get('CHARIOX_BROWSER_DISPLAY_OPENH264_ADAPTER'):
                    import importlib.util
                    spec=importlib.util.spec_from_file_location('native_openh264',Path(__file__).with_name('kernel-browser-openh264.py'));module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
                    codec=module.NativeOpenH264(frame.width,frame.height,int(request['bitrate']*.45))
                else:codec = av.CodecContext.create('libvpx' if selected=='vp8' else 'libvpx-vp9' if selected.startswith('vp09') else software_h264, 'w')
                codec.width, codec.height = frame.width, frame.height
                codec.pix_fmt = 'yuv420p'
                codec.time_base = Fraction(1,60)
                codec.framerate = Fraction(60,1)
                # Reserve nested base64, authenticated envelopes and credit replies.
                codec.bit_rate = int(request['bitrate'] * .45)
                codec.thread_count = 4
                codec.options = {'row-mt':'1', 'tile-columns':'2', 'frame-parallel':'1', 'deadline': 'realtime', 'cpu-used': '8', 'lag-in-frames': '0',
                                 'g': '60', 'error-resilient': '1', 'undershoot-pct': '95',
                                 'overshoot-pct': '5', 'bufsize': str(int(request['bitrate'] * .1)),
                                 'minrate': '0', 'maxrate': str(codec.bit_rate),
                                 'rc_init_occupancy': str(int(request['bitrate'] * .05)),
                                 'max-intra-rate': '200', 'qmin': '4', 'qmax': '48', 'crf':'28'}
                if selected=='vp8':codec.options={'deadline':'realtime','cpu-used':'8','lag-in-frames':'0','g':'120','error-resilient':'1'}
                if selected.startswith('avc1') and software_h264=='libopenh264':codec.options={'profile':'constrained_baseline','allow_skip_frames':'0','rc_mode':'bitrate','g':'120'}
                elif selected.startswith('avc1'):
                    # MP-08/MP-10: Selkies/pixelflux CBR at the negotiated rate,
                    # 1.5-frame VBV, infinite GOP; IDR only on an explicit reset.
                    codec.bit_rate=int(request['bitrate'])
                    codec.options={'forced-idr':'1','preset':'ultrafast','tune':'zerolatency','profile':'baseline',
                     'level':'5.1','g':'-1','bf':'0',
                     'x264-params':f'keyint=infinite:scenecut=0:sync-lookahead=0:repeat-headers=1:annexb=1:rc-lookahead=0:bitrate={codec.bit_rate//1000}:vbv-maxrate={codec.bit_rate//1000}:vbv-bufsize={max(32,codec.bit_rate*3//120000)}:filler=0'}
                configuration, sequence = config, 0
                mask_guard=protection.DecodedMaskGuard(selected) if regions else None
            frame = frame.reformat(format='yuv420p')
            # PNG decoders mark every input as I; clear that hint for inter prediction.
            frame.pict_type = av.video.frame.PictureType.I if request.get('reset') else av.video.frame.PictureType.NONE
            # Image demuxers supply their own timebase (often 25fps). Its PTS must
            # not be rebased into the negotiated 30fps video rate-control clock.
            frame.time_base = codec.time_base
            frame.pts = sequence
            packets = list(codec.encode(frame))
            if len(packets) != 1:
                raise ValueError('one realtime packet required')
            sequence += 1
            if mask_guard and not mask_guard.safe(bytes(packets[0]),regions,frame.width,frame.height,source_width=source_width,source_height=source_height):
                # Never publish an uncertain reconstruction, or a descendant
                # of its dropped reference. Exact repair may still settle.
                codec,configuration,mask_guard=None,None,None
                print(json.dumps({'dropped':True}),flush=True)
                continue
            print(json.dumps({'data_base64': base64.b64encode(bytes(packets[0])).decode(),
                              'key': packets[0].is_keyframe if not selected.startswith('avc1') else h264_idr(bytes(packets[0])),'backend':'vp8' if selected=='vp8' else 'vp9' if selected.startswith('vp09') else 'openh264' if software_h264=='libopenh264' else 'x264'}), flush=True)
        except Exception:
            codec, configuration = None, None
            print(json.dumps({'error': 'MD-DISPLAY: protected frame encode failed'}), flush=True)
        finally:
            if mapping is not None:mapping.close()
    if hardware is not None:hardware.close()
    if stripes is not None:stripes.close()

if __name__ == "__main__":
    main()
