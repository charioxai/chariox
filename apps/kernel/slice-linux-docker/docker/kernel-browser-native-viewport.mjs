import {setTimeout as delay} from 'node:timers/promises';

// MP-08/MP-10/MP-11: owned-window bounds are host DIPs; the measured
// protected reference must match the negotiated physical capture exactly.
export async function fitNativeViewport({connection,windowId,sessionId,width,height,hostScale=1,measure,wait=delay}) {
 const bounds={width:width/hostScale,height:height/hostScale+87};
 for(let n=0;n<50;n++){
  if(n%10===0)await connection.send('Browser.setWindowBounds',{windowId,bounds});
  const {bounds:actual}=await connection.send('Browser.getWindowBounds',{windowId});
  if(actual?.width===bounds.width&&actual?.height===bounds.height)break;
  await wait(20);
 }
 await connection.send('Page.bringToFront',{},sessionId);
 await wait(100);
 const actual=await measure();
 if(actual.width!==width||actual.height!==height)throw Error('native viewport geometry differed');
}
