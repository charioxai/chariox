"""MD-DISPLAY-02/04: owned encoder fallback, packet framing and signal guards."""
import base64
import importlib.util
import io
import json
import subprocess
import sys
import unittest
from pathlib import Path
import av
sys.dont_write_bytecode=True
path=Path(__file__).with_name('kernel-browser-encoder.py')
spec=importlib.util.spec_from_file_location('md_encoder',path)
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class EncoderTest(unittest.TestCase):
    def test_h264_key_flag_means_independent_IDR(self):
        import base64,re
        pixels=bytes(128*128*4)
        requests=[]
        for n in range(300):
            header={'raw':{'width':128,'height':128,'format':'bgr0','length':len(pixels)},'codec':'avc1.420033','bitrate':2000000,'reset':n in (0,65)}
            requests.append(json.dumps(header).encode()+b'\n'+pixels)
        run=subprocess.run([sys.executable,'-u',str(path)],input=b''.join(requests),stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=15)
        self.assertEqual(run.returncode,0)
        replies=[json.loads(line) for line in run.stdout.splitlines()]
        for n,reply in enumerate(replies):
            encoded=base64.b64decode(reply['data_base64'])
            nal_types=[part[0]&31 for part in re.split(b'\x00\x00\x01',encoded) if part]
            self.assertEqual(reply['key'],5 in nal_types,f'packet{n}: refresh point is not an independent IDR')
            if n in (0,65):self.assertTrue(reply['key'])
            if reply['key']:
                decoded=av.CodecContext.create('h264','r').decode(av.Packet(encoded))
                self.assertEqual(len(decoded),1)

    def test_motion_scales_only_software_video_and_keeps_exact_inputs_native(self):
        import base64
        pixels=bytes(2560*1600*4)
        for motion,expected in [(True,(1280,800)),(False,(2560,1600))]:
            header={'raw':{'width':2560,'height':1600,'format':'bgr0','length':len(pixels),'motion':motion},'codec':'avc1.420033','bitrate':2000000,'reset':True}
            run=subprocess.run([sys.executable,'-u',str(path)],input=json.dumps(header).encode()+b'\n'+pixels,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10)
            self.assertEqual(run.returncode,0)
            reply=json.loads(run.stdout)
            decoded=av.CodecContext.create('h264','r').decode(av.Packet(base64.b64decode(reply['data_base64'])))
            self.assertEqual((decoded[0].width,decoded[0].height),expected)

    def test_1080p_motion_scales_and_settle_stays_native(self):
        # MP-08/MP-10: decoded moving pixels reduce work; exact input stays native.
        import base64,os
        pixels=bytes(1920*1080*4)
        for motion,expected in [(True,(1280,720)),(False,(1920,1080))]:
            header={'raw':{'width':1920,'height':1080,'format':'bgr0','length':len(pixels),'motion':motion},'codec':'avc1.420033','bitrate':8000000,'reset':True}
            run=subprocess.run([sys.executable,'-u',str(path)],input=json.dumps(header).encode()+b'\n'+pixels,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10,env={**os.environ,'CHARIOX_BROWSER_DISPLAY_SOFTWARE':'1'})
            self.assertEqual(run.returncode,0)
            reply=json.loads(run.stdout)
            decoded=av.CodecContext.create('h264','r').decode(av.Packet(base64.b64decode(reply['data_base64'])))
            self.assertEqual((decoded[0].width,decoded[0].height),expected)

    def test_MP11_private_mask_handoff_is_immutable_and_decoded_black(self):
        import base64,os,tempfile
        width,height=128,128
        original=bytes([210,180,160,0])*(width*height)
        regions=[dict(x=40.5,y=40.5,width=30,height=30),dict(x=-100,y=0,width=20,height=20)]
        with tempfile.TemporaryDirectory(prefix='mp23-private-mask-') as root:
            file=Path(root)/'raster';file.write_bytes(original);file.chmod(0o600)
            header=dict(raw=dict(width=width,height=height,format='bgr0',length=len(original),shared=dict(path=str(file),length=len(original)),mask_shared=True),
                        operation='stripes',protected_regions=regions,codec='avc1.420033',bitrate=8000000,reset=True)
            # Invalid metadata must fail without retaining a mmap export or
            # corrupting a following admitted request on the same helper.
            invalid={**header,'protected_regions':[dict(x=float('nan'),y=0,width=5,height=5)]}
            run=subprocess.run([sys.executable,'-u',str(path)],input=(json.dumps(invalid)+'\n'+json.dumps(header)+'\n').encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10)
            replies=[json.loads(line) for line in run.stdout.splitlines()]
            self.assertIn('error',replies[0]);self.assertEqual(run.returncode,0)
            self.assertEqual(file.read_bytes(),original)
            self.assertEqual(len(replies[1]['stripes']),8)
            checked=0
            for row in replies[1]['stripes']:
                frame=av.CodecContext.create('h264','r').decode(av.Packet(base64.b64decode(row['data_base64'])))[0].reformat(format='rgb24')
                plane=bytes(frame.planes[0]);stride=frame.planes[0].line_size
                for y in range(max(41,row['y']),min(70,row['y']+row['height'])):
                    checked+=1
                    self.assertLessEqual(max(plane[(y-row['y'])*stride+41*3:(y-row['y'])*stride+70*3]),32)
                self.assertGreater(max(plane[:20*3]),100,'unrelated content stays visible')
            self.assertEqual(checked,29)

    def test_MP08_MP11_public_desktop_border_remains_video(self):
        width,height=1280,800
        pixels=bytearray(width*height*4)
        for y in range(507):
            for x in range(642):pixels[(y*width+x)*4:(y*width+x)*4+3]=bytes([255 if (x+y)%2 else 0])*3
        # A masked popup cuts into a public native editor beside an opaque browser.
        for y in range(90,200):pixels[(y*width+258)*4:(y*width+642)*4]=bytes(384*4)
        regions=[dict(x=0,y=507,width=1280,height=293),dict(x=642,y=0,width=638,height=507),dict(x=258,y=90,width=384,height=110)]
        header=dict(raw=dict(width=width,height=height,format='bgr0',length=len(pixels),motion=True),protected_regions=regions,codec='avc1.420033',bitrate=4000000,reset=True)
        run=subprocess.run([sys.executable,'-u',str(path)],input=json.dumps(header).encode()+b'\n'+pixels,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=15)
        self.assertEqual(run.returncode,0);reply=json.loads(run.stdout)
        self.assertIn('data_base64',reply,'MP-08 protected native window border must not turn video into a PNG fallback')
        frame=av.CodecContext.create('h264','r').decode(av.Packet(base64.b64decode(reply['data_base64'])))[0].reformat(format='rgb24')
        plane=bytes(frame.planes[0]);stride=frame.planes[0].line_size
        for region in regions:
            for y in range(region['y'],region['y']+region['height']):self.assertLessEqual(max(plane[y*stride+region['x']*3:y*stride+(region['x']+region['width'])*3]),32)
        self.assertGreater(max(plane[10*stride+10*3:10*stride+30*3]),64,'MP-08 unrelated public desktop remains visible')
    def test_MP11_unmasked_raw_input_still_refused(self):
        pixels=bytes([255])*128*128*4
        header=dict(raw=dict(width=128,height=128,format='bgr0',length=len(pixels)),protected_regions=[dict(x=20,y=20,width=40,height=40)],codec='avc1.420033',bitrate=4000000,reset=True)
        run=subprocess.run([sys.executable,'-u',str(path)],input=json.dumps(header).encode()+b'\n'+pixels,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=15)
        self.assertEqual(run.returncode,0);self.assertIn('error',json.loads(run.stdout),'MP-11 padding cannot grant unmasked input')

    def test_unsafe_signals(self):
        for pid in [None,0,1,-1,-2,float('nan'),2.5]:
            class Child:
                def poll(self):self.fail('must reject before signal')
                def kill(self):raise AssertionError('unsafe signal')
            child=Child();child.pid=pid
            encoder=module.VaapiEncoder();encoder.child=child
            with self.assertRaises(ValueError):encoder.abort()

    def test_framed_persistent_output_decodes_without_next_input(self):
        # CPU-backed FFmpeg exercises identical owned NUT framing and timeout
        # code, without pretending this builder has a hardware encoder.
        encoder=module.VaapiEncoder()
        encoder.child=subprocess.Popen(['ffmpeg','-v','error','-f','rawvideo','-pixel_format','bgr0',
            '-video_size','1280x800','-framerate','60','-i','pipe:0','-an','-c:v','libx264',
            '-preset','ultrafast','-tune','zerolatency','-bf','0','-pix_fmt','yuv420p',
            '-x264-params','repeat-headers=1:annexb=1','-f','nut','-flush_packets','1','pipe:1'],
            stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
        encoder.configuration=(1280,800,2000000)
        decoder=av.CodecContext.create('h264','r')
        try:
            frame=av.VideoFrame(1280,800,'bgr0');frame.planes[0].update(bytes(1280*800*4))
            for n in range(3):
                packet=encoder.encode(frame,2000000)
                frames=decoder.decode(av.Packet(bytes(packet)))
                self.assertEqual(len(frames),1);self.assertEqual((frames[0].width,frames[0].height),(1280,800))
                self.assertEqual(packet.is_keyframe,n==0)
        finally:encoder.close()
        self.assertIsNone(encoder.child)

    def test_unavailable_vaapi_falls_back_to_independent_software_packets(self):
        # The unavailable/denied VAAPI device on builder2 must not break the
        # selected H.264 contract. Capability is proven by packet decoding.
        pixels=bytes(1280*800*4)
        header={'raw':{'width':1280,'height':800,'format':'bgr0','length':len(pixels)},'codec':'avc1.420033','bitrate':2000000,'reset':True}
        run=subprocess.run([sys.executable,'-u',str(path)],input=json.dumps(header).encode()+b'\n'+pixels,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10)
        self.assertEqual(run.returncode,0)
        result=json.loads(run.stdout)
        import base64
        self.assertTrue(result['key'])
        decoded=av.CodecContext.create('h264','r').decode(av.Packet(base64.b64decode(result['data_base64'])))
        self.assertEqual(len(decoded),1)

if __name__=='__main__':unittest.main()
