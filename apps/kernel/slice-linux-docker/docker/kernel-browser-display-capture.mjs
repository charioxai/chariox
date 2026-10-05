// MD-DISPLAY-02/04: a protected thumbnail locates likely changes, never leaves
// the kernel, and never supplies displayed pixels. Changed rectangles are read
// at native DPR. An unchanged thumbnail ALWAYS triggers full protected readback
// to verify high-frequency detail it can miss. Large changes use full capture.
import { decodePng, encodePng } from './kernel-browser-pixels.mjs';
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
  const width = Math.min(1280,Math.ceil((right+1)/scale)+8)-x, height = Math.min(800,Math.ceil((bottom+1)/scale)+8)-y;
  return width * height <= 1280 * 800 * .15 ? { x,y,width,height,scale:1 } : null;
}
export class DisplayCapture {
  constructor(capture, scale, timing = () => {}, now = () => performance.now()) { this.capture = capture; this.scale = scale; this.timing = timing; this.now=now; this.tasks=new Set(); this.invalidate(); }
  invalidate() { this.prefetched=null; this.epoch=(this.epoch??0)+1; this.previous = null; this.preview = null; this.document = null; this.policy = null; this.needsVerification = false; this.inputEpoch = null; this.verifiedAt = -Infinity; this.source = null; this.motion=false; this.motionData=null; this.stableAt=null; }
  // One private full-viewport capture can overlap the serial codec operation.
  // It has no egress authority and is reusable only under the next admitted
  // read with the same policy, document, input epoch and short age bound.
  prefetch(tab,policy,clip) {
    if(!this.motion || !clip || this.tasks.size) return;
    const epoch=this.epoch,at=this.now();
    const task=this.capture(clip).then(source=> {
      if(this.epoch===epoch) this.prefetched={source,policy,document:tab.document_id,input:tab.input_epoch,at};
    }).catch(()=>{}).finally(()=>this.tasks.delete(task));
    this.tasks.add(task);
  }
  async close() { this.invalidate(); await Promise.allSettled([...this.tasks]); }
  async next(tab, policy, reusable, forceFull = false, motionClip = null) {
    if (!reusable || this.document !== tab.document_id || this.policy !== policy) this.invalidate();
    const capture = async clip => {
      const source = await this.capture(clip);
      if (source.document_id !== tab.document_id || source.tab_id !== tab.tab_id) { this.invalidate(); throw Error('MD-DISPLAY: capture binding changed'); }
      return source;
    };
    if (motionClip && this.motion) {
      const saved=this.prefetched;this.prefetched=null;
      const source=saved && saved.policy===policy && saved.document===tab.document_id &&
        saved.input===tab.input_epoch && this.now()-saved.at<100 ? saved.source : await capture(motionClip);
      if(source.document_id !== tab.document_id || source.tab_id !== tab.tab_id) {this.invalidate();throw Error('MD-DISPLAY: prefetch binding changed');}
      if(source.data_base64 !== this.motionData) this.stableAt=this.now();
      this.motionData=source.data_base64;
      if(this.now()-(this.stableAt??this.now()) < 150) return {...source,motion:true};
      // The entire motion viewport has stopped changing. Verify at native DPR
      // before attempting any exact tile; never upscale a lossless repair.
      motionClip=null; forceFull=true;
    }
    const verify = forceFull || (this.needsVerification && tab.input_epoch === this.inputEpoch);
    const previewSource = verify && !motionClip ? null : await capture({ x:motionClip?.x??0,y:motionClip?.y??0,width:1280,height:800,scale:factor });
    let at = timestamp();
    const preview = previewSource ? decodePng(previewSource.data_base64,this.scale) : this.preview;
    if (previewSource && (preview.width !== 1280*this.scale*factor || preview.height !== 800*this.scale*factor)) throw Error('MD-DISPLAY: preview geometry changed');
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
      source = { ...source, pixels, dirty_clip:clip, full_size_hint:this.fullSize,
        data_base64:() => encodePng(pixels.width,pixels.height,pixels.pixels) };
    } else {
      source = await capture(null); at = timestamp();
      pixels = decodePng(source.data_base64,this.scale);
      if (pixels.width !== 1280*this.scale || pixels.height !== 800*this.scale) throw Error('MD-DISPLAY: full geometry changed');
      this.timing('full_source_decode',at); this.fullSize = source.data_base64.length; this.verifiedAt=this.now();
      source = { ...source, pixels };
    }
    this.previous = pixels; this.preview = preview; this.document = tab.document_id; this.policy = policy;
    this.needsVerification = Boolean(clip); this.inputEpoch = tab.input_epoch;
    this.source=source; return source;
  }
}
