import {setTimeout as delay} from 'node:timers/promises';
import {chromeBandHeight} from './kernel-browser-geometry.mjs';

// MP-08/MP-10/MP-11: owned-window bounds are host DIPs; the measured
// protected reference must match the negotiated physical capture exactly.
export async function fitNativeViewport({connection,windowId,sessionId,width,height,hostScale=1,measure,wait=delay}) {
 if(![width,height].every(n=>Number.isSafeInteger(n)&&n>0)||![1,2].includes(hostScale))throw Error('native viewport geometry invalid');
 const initial={width:width/hostScale,height:height/hostScale+chromeBandHeight},bounds={...initial};
 for(let attempt=0;attempt<4;attempt++){
  let settled=false;
  for(let n=0;n<50;n++){
   if(n%10===0)await connection.send('Browser.setWindowBounds',{windowId,bounds});
   const {bounds:actual}=await connection.send('Browser.getWindowBounds',{windowId});
   if(actual?.width===bounds.width&&actual?.height===bounds.height){settled=true;break;}
   await wait(20);
  }
  if(!settled)throw Error('native viewport geometry unsettled');
  await connection.send('Page.bringToFront',{},sessionId);
  await wait(100);
  const actual=await measure();
  if(actual.width===width&&actual.height===height)return;
  if(![actual.width,actual.height].every(n=>Number.isSafeInteger(n)&&n>0))break;
  const next={width:Math.round(bounds.width+(width-actual.width)/hostScale),height:Math.round(bounds.height+(height-actual.height)/hostScale)};
  // Decorations are a small, bounded correction, never a substitute for
  // proving the surface. Large or oscillating mismatches refuse capture.
  if(Math.abs(next.width-initial.width)>64||Math.abs(next.height-initial.height)>128)break;
  Object.assign(bounds,next);
 }
 throw Error('native viewport geometry differed');
}
