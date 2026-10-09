// MP-08/MP-10: supplementary runtime bitrate regression, using the installed
// OpenH264 binary and production codec sources. No kernel build is required.
import {execFileSync} from 'node:child_process';
import {mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';

const source=fileURLToPath(new URL('../kernel/src/display_native/',import.meta.url));
const harness=fileURLToPath(new URL('./native-codec-rate.c',import.meta.url));
const includes=(process.env.CHARIOX_NATIVE_DISPLAY_INCLUDE??'').split(':').filter(Boolean).flatMap(p=>['-I',p]);
const libraries=(process.env.CHARIOX_NATIVE_DISPLAY_LIB??'').split(':').filter(Boolean).flatMap(p=>['-L',p]);
const root=mkdtempSync(join(tmpdir(),'chariox-native-rate-'));
const cc=(...args)=>execFileSync(process.env.CC??'cc',['-O2',...args],{stdio:'inherit'});
try{
 cc(...includes,'-Dcx_openh264_open=audited_openh264_open','-Dcx_openh264_rate=audited_openh264_rate',
    '-c',join(source,'codec.c'),'-o',join(root,'codec.o'));
 cc('-c',join(source,'openh264.c'),'-o',join(root,'openh264.o'));
 cc(...includes,harness,join(root,'codec.o'),join(root,'openh264.o'),...libraries,
    '-Wl,-Bstatic','-lyuv','-Wl,-Bdynamic','-ldl','-lpthread','-lm','-o',join(root,'regression'));
 const env={...process.env,CHARIOX_BROWSER_DISPLAY_SOFTWARE:'1'};
 delete env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER;
 const missing={...env,CHARIOX_BROWSER_DISPLAY_OPENH264:join(root,'missing-openh264.so.8')};
 execFileSync(join(root,'regression'),['missing'],{stdio:'inherit',env:missing});
 execFileSync(join(root,'regression'),['x264'],{stdio:'inherit',env:{...missing,CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER:'libx264'}});
 execFileSync(join(root,'regression'),[],{stdio:'inherit',env:{...env,CHARIOX_NATIVE_CODEC_PACKET_DIR:root}});
 // MP-08/MP-10: independently decode every reduced key and dependent frame,
 // including repeated reset/SPS sequences, with the installed FFmpeg codec.
 for(const [width,height] of [[1920,720],[2560,800]]) {
  execFileSync(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON??'python3',['-c',
   'import av,sys; frames=list(av.open(sys.argv[1],format="h264").decode(video=0)); assert len(frames)==6; assert all((f.width,f.height)==(1280,int(sys.argv[2])) for f in frames); print("MP-08/MP-10 independent noisy H264 decode PASS",len(frames),frames[0].width,frames[0].height)',
   join(root,`noise-${width}.h264`),String(height)],{stdio:'inherit'});
 }
}finally{rmSync(root,{recursive:true,force:true});}
