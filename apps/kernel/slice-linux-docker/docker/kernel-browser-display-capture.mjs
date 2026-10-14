import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-DISPLAY-02/04: a protected thumbnail locates likely changes, never leaves
// the kernel, and never supplies displayed pixels. Changed rectangles are read
// at native DPR. An unchanged thumbnail ALWAYS triggers full protected readback
// to verify high-frequency detail it can miss. Large changes use full capture.
import { decodePng, encodePng, maskPixels, displayMaskRegions, displayFullMaskRegions } from './kernel-browser-pixels.mjs';
import { timestamp } from './kernel-browser-timing.mjs';
const factor = 1 / 8;
export function changedClip(before, after, scale) {
  if (before.width !== after.width || before.height !== after.height) return null;
  let left = after.width, top = after.height, right = -1, bottom = -1;
  for (let y = 0; y < after.height; y++) for (let x = 0; x < after.width; x++) {
    const offset = (y * after.width + x) * 4;
    if (before.pixels.readUInt32LE(offset) !== after.pixels.readUInt32LE(offset)) {
      left = Math.min(left,x); right = Math.max(right,x); top = Math.min(top,y); bottom = Math.max(bottom,y);
    }
  }
  if (right < 0) return null;
  const x = Math.max(0,Math.floor(left / scale)-8), y = Math.max(0,Math.floor(top / scale)-8);
  const width = Math.min(after.width/scale,Math.ceil((right+1)/scale)+8)-x, height = Math.min(after.height/scale,Math.ceil((bottom+1)/scale)+8)-y;
  return width * height <= after.width*after.height/scale**2*.15 ? { x,y,width,height,scale:1 } : null;
}
export class DisplayCapture {
  constructor(capture, scale, timing = () => {}, now = () => performance.now()) { this.capture = capture; this.scale = scale; this.timing = timing; this.now=now; this.invalidate(); }
  invalidate() { this.previous = null; this.preview = null; this.document = null; this.policy = null; this.needsVerification = false; this.inputEpoch = null; this.verifiedAt = -Infinity; this.source = null; this.native=null; this.motion=false; this.motionData=null; this.stableAt=null; }
  verified(tab,policy,epoch) {
    return !this.motion && !this.needsVerification && this.source && this.document===tab.document_id && this.policy===policy && this.inputEpoch===epoch && this.now()-this.verifiedAt<250 ? this.source : null;
  }
  async next(tab, policy, reusable, forceFull = false, motionClip = null, scrollActive = false) {
    if (!reusable || this.document !== tab.document_id || this.policy !== policy) this.invalidate();
    const capture = async clip => {
      const source = await this.capture(clip);
      if (source.document_id !== tab.document_id || source.tab_id !== tab.tab_id) { this.invalidate(); throw Error('MD-DISPLAY: capture binding changed'); }
      return source;
    };
    // Repeating paragraphs can look unchanged in a tiny thumbnail while a
    // wheel is moving the whole document. Only admitted source input supplies
    // this hint; it selects a complete protected viewport, never inferred pixels.
    if(motionClip && scrollActive) {
      const source=await capture(motionClip);
      this.previous=null;this.document=tab.document_id;this.policy=policy;
      this.inputEpoch=tab.input_epoch;this.motion=true;this.motionData=source.data_base64;this.stableAt=this.now();
      return {...source,motion:true};
    }
    if (motionClip && this.motion) {
      const source=await capture(motionClip);
      if(source.data_base64 !== this.motionData) this.stableAt=this.now();
      this.motionData=source.data_base64;
      if(this.now()-(this.stableAt??this.now()) < 300) return {...source,motion:true};
      // The entire motion viewport has stopped changing. Verify at native DPR
      // before attempting any exact tile; never upscale a lossless repair.
      motionClip=null; forceFull=true;
    }
    const verify = forceFull || (this.needsVerification && tab.input_epoch === this.inputEpoch);
    const previewSource = verify && !motionClip ? null : await capture({ x:motionClip?.x??0,y:motionClip?.y??0,width:geometry.width,height:geometry.height,scale:factor });
    let at = timestamp();
    const preview = previewSource ? decodePng(previewSource.data_base64,this.scale) : this.preview;
    if (previewSource && (preview.width !== geometry.width*this.scale*factor || preview.height !== geometry.height*this.scale*factor)) throw Error('MD-DISPLAY: preview geometry changed');
    this.timing('preview_decode',at);
    // Already verified pixels may be reused briefly only on an unchanged
    // native thumbnail, unchanged input epoch and empty protection registry.
    // Fine detail is verified at least every250ms, and every admitted input
    // or crop forces its next full readback. No inferred pixels leave capture.
    if (!verify && this.source && this.preview?.pixels.equals(preview?.pixels) &&
        tab.input_epoch === this.inputEpoch && policy.values?.length === 0 &&
        this.now()-this.verifiedAt < 250) return this.source;
    const damage = this.preview && preview && changedClip(this.preview,preview,this.scale*factor);
    const clip = !verify && this.previous && damage;
    let source, pixels;
    const changing = this.preview && preview && !this.preview.pixels.equals(preview.pixels);
    if (motionClip && changing && (!damage || this.motion)) {
      source=await capture(motionClip);
      // This is a complete protected viewport at reduced motion resolution,
      // never a dirty-region approximation. Idle verification stays native DPR.
      this.previous=null;this.preview=preview;this.document=tab.document_id;this.policy=policy;
      this.inputEpoch=tab.input_epoch;this.motion=true;this.motionData=source.data_base64;this.stableAt=this.now();
      return {...source,motion:true};
    }
    this.motion=false;
    if (clip) {
      source = await capture(clip); at = timestamp();
      const crop = decodePng(source.data_base64,this.scale);
      if (crop.width !== clip.width*this.scale || crop.height !== clip.height*this.scale) throw Error('MD-DISPLAY: crop geometry changed');
      pixels = { ...this.previous, pixels:Buffer.from(this.previous.pixels) };
      for (let row = 0; row < crop.height; row++) crop.pixels.copy(pixels.pixels,
        ((clip.y*this.scale+row)*pixels.width+clip.x*this.scale)*4,row*crop.width*4,(row+1)*crop.width*4);
      this.timing('crop_decode_merge',at);
      // The merged raster is full size. Crop-relative masks cannot bind its
      // encoder input/output; retain fields outside the crop as well, and mask
      // older pixels under a field that appeared outside the crop.
      if(source[displayMaskRegions]&&!source[displayFullMaskRegions])throw Error('MP-11: full raster masks unavailable');
      maskPixels(pixels,(source[displayFullMaskRegions]??[]).map(r=>[r.x,r.y,r.width,r.height]));
      source = { ...source, pixels, dirty_clip:clip, full_size_hint:this.fullSize,
        [displayMaskRegions]:source[displayFullMaskRegions]??[],
        data_base64:() => encodePng(pixels.width,pixels.height,pixels.pixels) };
    } else {
      source = await capture(null); at = timestamp();
      // Comparing a complete protected PNG is an exact verification, unlike
      // thumbnail equality. Reuse its already-decoded immutable pixel buffer
      // only when every byte and the policy/document binding still matches.
      if(this.native?.data === source.data_base64) pixels=this.native.pixels;
      else {
        pixels = decodePng(source.data_base64,this.scale);
        this.timing('full_source_decode',at);
        this.native={data:source.data_base64,pixels};
      }
      if (pixels.width !== geometry.width*this.scale || pixels.height !== geometry.height*this.scale) throw Error('MD-DISPLAY: full geometry changed');
      this.fullSize = source.data_base64.length; this.verifiedAt=this.now();
      source = { ...source, pixels };
    }
    this.previous = pixels; this.preview = preview; this.document = tab.document_id; this.policy = policy;
    this.needsVerification = Boolean(clip); this.inputEpoch = tab.input_epoch;
    this.source=source; return source;
  }
}
