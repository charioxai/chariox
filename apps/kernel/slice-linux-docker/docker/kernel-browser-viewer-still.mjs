// Row 341 (A), MP-08/MP-11: a DPR2 viewer of a page that runs at DPR1 (a headed desktop at scale 1) saw the whole-page
// video's picture at rest upscaled 2x, soft next to the sharp DOM mirror. Once the page is quiet the exact still is taken at
// the viewer's density instead: Chrome re-rasterizes the visible viewport through a clip scale (as #1676's region stills
// do); the page's own density never changes and live video stays at it.
import {createHash} from 'node:crypto';
import {displayMaskRegions,displayFullMaskRegions} from './kernel-browser-pixels.mjs';
// The page picture a viewer still answers: a hash of the native raster's pixels, once per raster. A headed window repaints
// around every dense capture (it is enlarged for it), and the native frame after that repaint is a new frame of the same
// pixels; frame serials and the native worker's signature (its serial) cannot tell. Without a raster: the source's
// fingerprint, else its serial.
const hashes=new WeakMap();
export function viewerPixels(sample){
  const raw=sample?.raw;
  if(raw&&typeof raw==='object'){
    if(hashes.has(raw))return hashes.get(raw);
    try{const bytes=raw.pixels;if(bytes?.length){const hash=`raster:${createHash('sha1').update(bytes).digest('hex')}`;hashes.set(raw,hash);return hash;}}catch{}
  }
  return sample?.signature??`serial:${sample?.serial}`;
}
// A viewer above the page's density whose canvas is not at that density yet, unless this exact picture refused a still.
export function viewerStillPending(stream,{source,document,epoch,policy,sample}){
  const page=stream.device_scale_factor;
  if(!((stream.viewer_scale??page)>page&&stream.previousDensity!==stream.viewer_scale))return false;
  const c=stream.viewerStill;
  return !(c&&c.still===null&&c.source===source&&c.document===document&&c.epoch===epoch&&c.policy===policy&&c.pixels===viewerPixels(sample));
}
// One display frame (the 1 MiB egress bound) with room for its envelope: a full PNG, never tiles, changes the canvas size.
const FRAME_BYTES=1024*1024-64*1024;
export async function viewerDensityStill({host,tab,compositor,stream}){
  // Each outcome is a display-timing event, so a live run shows why a viewer kept the page-density picture.
  const none=reason=>{host.timing?.event?.('viewer_density_still',{outcome:reason});return null;};
  const page=host.scales.get(tab.tab_id)??1,viewer=stream.viewer_scale??page;
  if(!(viewer>page)||![1,2].includes(viewer))return none(viewer>page?'density':'page_density');
  const width=stream.css_width,height=stream.css_height;
  if(![width,height].every(v=>Number.isSafeInteger(v)&&v>0))return none('size');
  const {connection,sessionId}=compositor?.connection?compositor:await host.browser.resolvePageTarget(tab.target_id);
  // The visible viewport in page coordinates; an unzoomed one only. One that moved during the capture is `busy`:
  // the refiner drops this capture and asks again when the page is quiet.
  const origin=async()=>(await connection.send('Page.getLayoutMetrics',{},sessionId))?.cssVisualViewport;
  const before=await origin();
  if(!(before?.scale===1&&Number.isFinite(before.pageX)&&Number.isFinite(before.pageY)))return none('zoomed');
  // MP-11: the protected, masked capture. Any Vault-protected field keeps the page-density path and its masking.
  const {protected_regions=[],...shot}=await host.screenshot(tab,{x:before.pageX,y:before.pageY,width,height,scale:viewer/page},true,'png',false,true);
  const after=await origin();
  if(after?.pageX!==before.pageX||after?.pageY!==before.pageY)throw Object.assign(Error('MP-08: viewport moved during the viewer-density still'),{busy:true});
  if(protected_regions.length)return none('protected');
  if(typeof shot.data_base64!=='string'||shot.data_base64.length>FRAME_BYTES)return none('over_frame');
  const png=Buffer.from(shot.data_base64,'base64');
  if(png.length<24||png.readUInt32BE(16)!==width*viewer||png.readUInt32BE(20)!==height*viewer)return none('dimensions');
  host.timing?.event?.('viewer_density_still',{outcome:'still',width:width*viewer,height:height*viewer,bytes:png.length});
  return {...shot,width:width*viewer,height:height*viewer,viewer_density:viewer,device_scale_factor:viewer,[displayMaskRegions]:[],[displayFullMaskRegions]:[]};
}
