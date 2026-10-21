// MP-08/MP-10/MP-11: decode production packets independently; flat source
// pixels must not acquire dark horizontal seams at internal stripe crops.
import {execFileSync} from 'node:child_process';
import {mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
const source=fileURLToPath(new URL('../kernel/src/display_native/',import.meta.url));
const includes=(process.env.CHARIOX_NATIVE_DISPLAY_INCLUDE??'').split(':').filter(Boolean).flatMap(p=>['-I',p]);
const libraries=(process.env.CHARIOX_NATIVE_DISPLAY_LIB??'').split(':').filter(Boolean).flatMap(p=>['-L',p]);
const root=mkdtempSync(join(tmpdir(),'chariox-stripe-geometry-'));
const cc=(...args)=>execFileSync(process.env.CC??'cc',['-O2',...args],{stdio:'inherit'});
try {
 cc(...includes,'-c',join(source,'codec.c'),'-o',join(root,'codec.o'));
 cc('-c',join(source,'openh264.c'),'-o',join(root,'openh264.o'));
 cc(...includes,fileURLToPath(new URL('./native-codec-stripe-geometry.c',import.meta.url)),join(root,'codec.o'),join(root,'openh264.o'),...libraries,'-Wl,-Bstatic','-lyuv','-Wl,-Bdynamic','-ldl','-lpthread','-lm','-o',join(root,'regression'));
 const env={...process.env,CHARIOX_BROWSER_DISPLAY_SOFTWARE:'1'};delete env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER;
 for(const height of [800,1600]) {
  execFileSync(join(root,'regression'),[String(height),root],{stdio:'inherit',env});
  execFileSync(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON??'python3',['-c',
   `import av,sys,numpy as np
from pathlib import Path
for file in sorted(Path(sys.argv[1]).glob(sys.argv[2]+'-row*.h264')):
 image=next(av.open(str(file),format='h264').decode(video=0)).to_ndarray(format='rgb24')
 middle=image[image.shape[0]//2].astype('int16')
 delta=max(float(np.abs(image[-k].astype('int16')-middle).mean()) for k in range(1,5))
 print('MP-08/MP-10/MP-11 independent internal stripe seam',file.name,delta,flush=True)
 assert delta<=1,'MP-08: native encoder padding darkened a flat stripe edge'
`,root,String(height)],{stdio:'inherit'});
 }
} finally {rmSync(root,{recursive:true,force:true})}
