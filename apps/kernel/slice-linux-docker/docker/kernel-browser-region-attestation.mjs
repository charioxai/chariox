// MP-08/MP-10/MP-11: prove a native frame's identity and origin using exact
// non-media pixels. Only the kernel observer supplies media boxes; these are
// comparison exclusions, never display/protection masks.
import {LinuxCapture,attestationMask,statusBand} from './kernel-browser-native.mjs';
import {NativeRegionProtection} from './kernel-browser-region-protection.mjs';
import {startNativeReadback,nativeReadbackFailure} from './kernel-browser-native-readback.mjs';
import {cropProtectedPng,decodePng,displayMaskRegions} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';

function mediaMask(boxes,width,height){
 const mask=new Uint8Array(width*height);
 for(const {x,y,width:w,height:h} of boxes){
  if(![x,y,w,h].every(Number.isFinite)||w<=0||h<=0)throw Error('MP-11: media attestation geometry');
  const left=Math.max(0,Math.floor(x)),top=Math.max(0,Math.floor(y)),right=Math.min(width,Math.ceil(x+w)),bottom=Math.min(height,Math.ceil(y+h));
  for(let row=top;row<bottom;row++)if(right>left)mask.fill(1,row*width+left,row*width+right);
 }
 return mask;
}
const ringWidth=3;
export function attestationPlan(region,media,scale){
 const boxes=media.map(({box})=>{
  if(!Array.isArray(box)||box.length!==4||!box.every(Number.isFinite)||box[2]<=0||box[3]<=0)throw Error('MP-11: media attestation geometry');
  return {x:box[0]*scale,y:box[1]*scale,width:box[2]*scale,height:box[3]*scale};
 });
 const project=rect=>boxes.map(b=>({...b,x:b.x-rect.left+region.contentRect.left,y:b.y-rect.top+region.contentRect.top}));
 let rect={left:region.contentRect.left+region.pageLeft,top:region.contentRect.top+region.pageTop,width:region.width,height:region.height};
 if(media.length){
  // Full-bleed crops need surrounding page pixels at the SAME authoritative
  // origin. The kernel-owned window/lease still binds full-bleed media when no page pixels remain.
  const page=region.contentRect;
  rect={left:page.left,top:page.top,width:page.width-page.width%2,height:page.height-page.height%2};
 }
 const visible=project(rect).map(b=>{const x=Math.max(0,b.x),y=Math.max(0,b.y),right=Math.min(rect.width,b.x+b.width),bottom=Math.min(rect.height,b.y+b.height);return{x,y,width:right-x,height:bottom-y}}).filter(b=>b.width>0&&b.height>0);
 return {rect,media:visible};
}
// `strips` are browser-owned bands (the status bubble, the scrollbar gutter): not page content, so media may
// reach them and the two rasters rasterize them differently; their pixels carry no identity proof (zeroed in
// both rasters) but never count as a protected region.
export function assertAttestationPixels(raw,reference,media,protectedRegions,strips=[]){
 if(!raw||raw.width!==reference.width||raw.height!==reference.height||raw.pixels.length!==raw.width*raw.height*4||reference.pixels.length!==raw.pixels.length)
  throw Error('MP-11: native region attestation geometry');
 const masked=mediaMask(media,raw.width,raw.height),protectedMask=attestationMask(protectedRegions,raw.width,raw.height),blankMask=attestationMask([...protectedRegions,...strips],raw.width,raw.height);
 const excluded=masked.reduce((n,v)=>n+v,0),total=masked.length;
 for(let p=0;p<total;p++)if(masked[p]&&protectedMask?.[p])throw Error('MP-11: protected media overlap');
 const ring=new Uint8Array(total),width=raw.width,height=raw.height;
 const visit=(x,y)=>{const p=y*width+x;if(masked[p])return;if(protectedMask?.[p])throw Error('MP-11: media attestation ring unavailable');ring[p]=1};
 for(const box of media){
  const l=Math.max(0,Math.floor(box.x)),t=Math.max(0,Math.floor(box.y)),r=Math.min(width,Math.ceil(box.x+box.width)),b=Math.min(height,Math.ceil(box.y+box.height));
  if(r<=l||b<=t)throw Error('MP-11: media attestation geometry');
  const el=Math.max(0,l-ringWidth),et=Math.max(0,t-ringWidth),er=Math.min(width,r+ringWidth),eb=Math.min(height,b+ringWidth);
  const edge=visit;
  for(let y=et;y<t;y++)for(let x=el;x<er;x++)edge(x,y);
  for(let y=t;y<b;y++){for(let x=el;x<l;x++)edge(x,y);for(let x=r;x<er;x++)edge(x,y)}
  for(let y=b;y<eb;y++)for(let x=el;x<er;x++)edge(x,y);
 }
 // Protection wins before the comparison in BOTH rasters, including a native
 // protection fence newer than the CDP sample. Those pixels prove no identity.
 const pixels=blankMask?Buffer.from(raw.pixels):raw.pixels,referencePixels=blankMask?Buffer.from(reference.pixels):reference.pixels;
 let anchors=0,protectedCount=0;
 for(let p=0;p<total;p++){
  const i=p*4;
  if(blankMask?.[p]){protectedCount++;pixels.fill(0,i,i+3);referencePixels.fill(0,i,i+3)}
  if(masked[p])continue;
  if(!blankMask?.[p])anchors++;
  if(referencePixels[i]!==pixels[i+2]||referencePixels[i+1]!==pixels[i+1]||referencePixels[i+2]!==pixels[i])
   throw Error('MP-11: native region attestation mismatch');
 }
 // Media size is not an admission boundary. The caller binds PID/XID, active tab, document and crop;
 // Vault masks independently fence every frame. Compare every available non-media pixel, even if none remain.
 return {excluded_pixels:excluded,page_pixels:total,proof_pixels:anchors,protected_pixels:protectedCount,ring_pixels:ring.reduce((n,v)=>n+v,0),media_boxes:media.length};
}
const SCROLL_GUTTER_DIP=16;
const browserStrips=(page,hostScale)=>[statusBand(page,hostScale),{x:page.width-SCROLL_GUTTER_DIP*hostScale,y:0,width:SCROLL_GUTTER_DIP*hostScale,height:page.height}];
export function regionProtection(source,left,top){
 const regions=new NativeRegionProtection(source.connection,source.sessionId,
  {policy:source.policy,browser:source.browser,tab:source.tab,scale:source.scale});
 const measure=regions.measure.bind(regions);
 regions.measure=async()=> (await measure()).map(([x,y,w,h])=>[x-left,y-top,w,h]);
 return regions;
}
export async function attestNativeRegion(source){
 if(typeof source.attestationState!=='function')throw Error('MP-11: region attestation binding unavailable');
 const before=await source.attestationState(),plan=attestationPlan(source.region,before.media,source.scale);
 const page=source.region.contentRect,left=plan.rect.left-page.left,top=plan.rect.top-page.top;
 // This temporary readback has no input or encoder and is never advertised.
 // It proves surrounding pixels using the same PID/XID, origin and live lease.
 const probe=new LinuxCapture(source);
 probe.valid=()=>!probe.closed&&source.valid();
 probe.validateCurrent=source.validateCurrent;
 probe.regions=regionProtection(source,left,top);
 probe.off=source.connection.subscribe(m=>probe.onCdp(m));
 try{
  await probe.regions.refresh();probe.poolRoot=await probe.createRasterPool();
  startNativeReadback(probe,{width:plan.rect.width,height:plan.rect.height,
   window_region:{...source.region.window_region,left:plan.rect.left,top:plan.rect.top}});
  if(!await probe.waitForCurrentSample())throw nativeReadbackFailure(probe,'MP-08: native region readback unavailable');
  const shot=await source.screenshot();
  const clip={x:left/source.scale,y:top/source.scale,width:plan.rect.width/source.scale,height:plan.rect.height/source.scale};
  const reference=decodePng(cropProtectedPng(shot.data_base64,clip,source.scale).data_base64,source.scale);
  const projected=r=>({...r,x:r.x-left,y:r.y-top});
  const protectedRegions=(shot[displayMaskRegions]??[]).map(projected);
  const raw=probe.latest?.raw;protectedRegions.push(...(raw?.[displayMaskRegions]??[]));
  const receipt=assertAttestationPixels(raw,reference,plan.media,protectedRegions,browserStrips(page,source.hostScale).map(projected));
  await assertCurrentDocument(source.connection,source.sessionId,source.tab.target_id,source.tab.document_id);
  const after=await source.attestationState();
  if(JSON.stringify(before)!==JSON.stringify(after)||!probe.valid()||!source.valid()||!await source.validateCurrent())
   throw Error('MP-11: native region attestation binding changed');
  source.attestationReceipt=Object.freeze(receipt);
  source.timing.event?.('native_region_attestation',receipt);
 }finally{await probe.close();}
}
