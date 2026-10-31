import {displayGeometryMatches,regionBindingCurrent,regionInputBinding,regionMinimumProtocolVersion,tabDisplayMinimumProtocolVersion,visiblePageMinimumProtocolVersion} from './region-binding.mjs';
import {StripePresenter,independentStripeCover} from './stripe-presenter.mjs';
import {ScrollPrediction} from './scroll-prediction.mjs';
import {WorkerVideoDecoder} from './decoder-worker.mjs';
import {VideoSink} from './video-sink.mjs';
import {PresentationQueue} from './presentation-queue.mjs';
// MD-DISPLAY-04: protocol 419 presentation only. Cloud supplies its existing
// admitted/encrypted kernel request and event adapter; never a Cloud media proxy.
export const minimumProtocolVersion = 466;
export const desktopMinimumProtocolVersion = 474;
export const pushProtocolVersion = 491;
// MP-08/MP-10/MP-11: this optional shared resumption path requires 525.
export const resumeProtocolVersion = 525;
// PERF_AUDIT 4.2.3: a transport that holds frames pushed before its registration response asks for
// the first frame at registration instead of after its first acknowledgement (one round trip).
export const initialPushProtocolVersion = 540;
const VP9='vp09.00.10.08';
const videoCodecs=['vp8','avc1.420033','vp09.00.50.08','vp09.00.40.08',VP9];
// Empty credits must not saturate a narrow link with control traffic. Admitted
// input and changed frames wake every parked slot without waiting for a timer.
export class IdleCredit {
  constructor(now=()=>performance.now()) { this.delay=0; this.parked=new Set();this.now=now;this.activeUntil=-Infinity;this.roundTrip=null; }
  observeRoundTrip(ms) { if(Number.isFinite(ms)&&ms>=0)this.roundTrip=this.roundTrip===null?ms:this.roundTrip*.8+ms*.2; }
  wake() { this.delay=0;this.activeUntil=this.now()+300;for(const wake of this.parked)wake(); }
  wait() {
    const active=this.now()<this.activeUntil;
    // Fast retries help a short local credit loop; on a slow link they add
    // encrypted control traffic and TCP loss exposure without hiding its RTT.
    const fast=active&&this.roundTrip!==null&&this.roundTrip<40;
    this.delay=Math.min(active?(fast?8:33):100,Math.max(active?8:32,this.delay*2));
    return new Promise(resolve=>{
      const wake=()=>{clearTimeout(timer);this.parked.delete(wake);resolve();};
      const timer=setTimeout(wake,this.delay);this.parked.add(wake);
    });
  }
}
// MP-08/MP-10: protocol 466 raw payload views. Decoder workers take a
// transferable copy so sibling segments of one event stay readable.
function bytes(data) {
  if (!(data instanceof Uint8Array) || data.byteLength < 1 || data.byteLength > 4 * 1024 * 1024) throw new Error('MD-DISPLAY: payload bound');
  return data.slice();
}
export const frameBytes = frame => 1024 + (frame.data?.byteLength ?? 0) +
  [...(frame.tiles ?? []), ...(frame.stripes ?? [])].reduce((n, segment) => n + (segment.data?.byteLength ?? 0), 0);
const independentFrame = frame => frame.kind === 'png' || frame.kind === 'video' && frame.key === true || independentStripeCover(frame);
const imageTypes = { png: 'image/png', webp: 'image/webp' };
// MP-08/MP-10: lossless scroll moves copy integer rectangles of the previous
// canvas (source y-dy). Every destination and source stays inside the frame.
function validMoves(frame) {
  if (frame.moves === undefined) return true;
  return frame.kind === 'tiles' && Array.isArray(frame.moves) && frame.moves.length >= 1 && frame.moves.length <= 64 &&
    frame.moves.every(m => Array.isArray(m) && m.length === 5 && m.every(Number.isSafeInteger) && m[0] >= 0 && m[1] >= 0 && m[2] >= 1 && m[3] >= 1 &&
      m[0] + m[2] <= frame.width && m[1] + m[3] <= frame.height && m[1] - m[4] >= 0 && m[1] - m[4] + m[3] <= frame.height);
}
export async function supportedCodecs() {
  const codecs = ['png'];
  if(globalThis.VideoDecoder)for(const codec of [...videoCodecs].reverse())if((await VideoDecoder.isConfigSupported({codec})).supported)codecs.unshift(codec);
  return [...codecs,'chariox-video-dependencies-v1',...(codecs.some(c=>c==='avc1.420033'||c==='vp8')?['chariox-stripes-v1']:[])];
}
export class BrowserDisplayPresenter {
  constructor(canvas, binding, onTiming = () => {}, warmDecoder = null, videoOptions = {}) {
    this.canvas = canvas; this.binding = binding; this.sequence = 0; this.documentId = null;
    this.busy = false; this.closed = false;
    this.onTiming = onTiming;
    // MP-08/MP-10/MP-11: request the low-latency compositor path before the
    // first crop/Chrome UI draw. Full-page and desktop streams retain their sink.
    if(videoOptions.direct)canvas.getContext?.('2d',{desynchronized:true});
    this.videoSink=new VideoSink(canvas,onTiming,videoOptions);this.prepared=new WeakMap();this.outcomes=new WeakMap();this.deferredVideo=new WeakMap();
    // Start the single module fetch while source navigation/capture is pending.
    if(warmDecoder&&!warmDecoder.closed)this.workerDecoder=warmDecoder;
    else if(globalThis.Worker&&globalThis.VideoDecoder){try{this.workerDecoder=new WorkerVideoDecoder();}catch{/* Codec admission/presentation owns worker failures. */}}
  }
  getWorkerDecoder(){if(!this.workerDecoder||this.workerDecoder.closed)this.workerDecoder=new WorkerVideoDecoder();return this.workerDecoder;}
  validate(frame,sequence) {
    if (frame.subscription_id !== this.binding.subscription_id || frame.generation !== this.binding.generation || frame.tab_id !== this.binding.tab_id || frame.sequence <= sequence) return false;
    if (!Number.isSafeInteger(frame.sequence) || ![1, 2].includes(frame.device_scale_factor) || frame.width !== frame.css_width * frame.device_scale_factor || frame.height !== frame.css_height * frame.device_scale_factor || !displayGeometryMatches(this.binding,frame) || typeof frame.document_id !== 'string' || !frame.document_id || frame.document_id.length > 256) throw new Error('MD-DISPLAY: invalid geometry/binding');
    return true;
  }
  prepareVideo(frame) {
    if(this.prepared.has(frame))return;
    const prepare=async()=>{
      if(this.closed||!this.validate(frame,this.decodeSequence??this.sequence))throw Error('MD-DISPLAY: rejected video binding');
      if(!validMoves(frame))throw Error('MD-DISPLAY: move geometry');
      if(frame.kind!=='video'||!videoCodecs.includes(frame.codec)||typeof frame.key!=='boolean')throw Error('MD-DISPLAY: unsupported frame');
      const sequence=this.decodeSequence??this.sequence,documentId=this.decodeDocumentId??this.documentId;
      if(!frame.key&&(this.videoSequence===null||sequence!==frame.sequence-1||documentId!==frame.document_id))throw Error('MD-DISPLAY: video base lost; subscribe afresh');
      this.decodeSequence=frame.sequence;this.decodeDocumentId=frame.document_id;
      if(!globalThis.Worker)return null;
      this.prediction?.restore();
      const decoder=this.getWorkerDecoder();this.videoSink.setup(decoder);
      const payload=bytes(frame.data),receivedAt=frame[Symbol.for('chariox.display.receivedAt')];
      if(Number.isFinite(receivedAt))this.onTiming('client_receive',receivedAt,{sequence:frame.sequence,receivedAt,decodeSubmittedAt:performance.timeOrigin+performance.now()});
      const result=await decoder.decode(frame,payload,8,true);
      if(this.closed){result.image.close();throw Error('MD-DISPLAY: presenter closed');}
      try{
        // MP-08/MP-10/MP-11: an opaque consumer copies this detached canvas
        // at the ordered cursor. Decode-ahead must not replace its pixels first.
        if(this.canvas.isConnected===false)this.deferredVideo.set(frame,{decodedAt:result.decodedAt});
        else await this.commitVideo(result,frame);
        return result.image;
      }catch(error){result.image.close();throw error;}
    };
    const output=prepare();output.catch(()=>{});this.prepared.set(frame,output);
  }
  async commitVideo(result,frame){
    const outcome=await this.videoSink.commit(result,frame);this.outcomes.set(frame,outcome);
    this.onTiming('client_decode',result.decodedAt,{sequence:frame.sequence,decodedAt:result.decodedAt,writtenAt:outcome.writtenAt,committedAt:outcome.committedAt});
    if(outcome.drawn&&outcome.writtenAt===undefined&&outcome.committedAt!==undefined)this.onTiming('client_present',outcome.committedAt,{sequence:frame.sequence,committedAt:outcome.committedAt});
    return outcome;
  }
  discardPrepared(frame){this.prepared.get(frame)?.then(image=>{this.deferredVideo.delete(frame);image?.close()},()=>{});}
  wasDrawn(frame){return this.outcomes.get(frame)?.drawn??this.didDraw;}
  async present(frame) {
    if (this.closed) {this.prepared.get(frame)?.then(image=>image?.close(),()=>{});return false;}
    if (this.busy) throw new Error('MD-DISPLAY: await presentation before granting next credit');
    if (!this.validate(frame,this.sequence)) return false;
    if (['tiles','stripes'].includes(frame.kind) && !independentStripeCover(frame) && (frame.base_sequence !== this.sequence || frame.document_id !== this.documentId)) throw new Error('MD-DISPLAY: repair base lost; subscribe afresh');
    if (!validMoves(frame)) throw new Error('MD-DISPLAY: move geometry');
    this.prediction?.restore();
    this.busy = true;
    const at = performance.timeOrigin + performance.now();
    const patch = ['tiles','stripes'].includes(frame.kind);
    if (patch && this.sequence>0 && (this.canvas.width !== frame.width || this.canvas.height !== frame.height)) { this.busy = false; throw new Error('MD-DISPLAY: repair canvas changed'); }
    if(frame.kind === 'png' && (!this.back || this.back.width !== frame.width || this.back.height !== frame.height))
      this.back=new OffscreenCanvas(frame.width,frame.height);
    const back=patch?null:this.back,context=back?.getContext('2d');
    const bitmaps = [];
    let videoFrame, stripes;
    try {
      if(frame.kind==='stripes'){
        if(this.documentId!==frame.document_id)this.stripeDecoder?.close();
        this.stripeDecoder??=new StripePresenter(()=>this.getWorkerDecoder());
        stripes=await this.stripeDecoder.decode(frame,bytes);
      }else if (frame.kind === 'tiles') {
        if (!Array.isArray(frame.tiles) || frame.tiles.length > 260) throw new Error('MD-DISPLAY: tile count');
        // MP-08/MP-10/MP-11: validate the complete packet before starting any
        // decode; four workers overlap independent images without unbounded jobs.
        for (const tile of frame.tiles) {
          if (![tile.x, tile.y, tile.width, tile.height].every(Number.isSafeInteger) || tile.x < 0 || tile.y < 0 || tile.width < 1 || tile.height < 1 || tile.width * tile.height > 2560 * 256 || tile.x + tile.width > frame.width || tile.y + tile.height > frame.height || !Object.hasOwn(imageTypes, tile.format)) throw new Error('MD-DISPLAY: tile geometry');
        }
        let next=0,decodeError;
        const decode=async()=>{
          while(!decodeError && next<frame.tiles.length){
            const index=next++,tile=frame.tiles[index];
            try{
              const bitmap=await createImageBitmap(new Blob([bytes(tile.data)],{type:imageTypes[tile.format]}));
              bitmaps[index]=bitmap;
              if(bitmap.width!==tile.width||bitmap.height!==tile.height)throw Error('MD-DISPLAY: tile dimensions');
            }catch(error){decodeError??=error;}
          }
        };
        // All in-flight decodes must settle before finally closes their images.
        await Promise.all(Array.from({length:Math.min(4,frame.tiles.length)},decode));
        if(decodeError)throw decodeError;
      } else if (frame.kind === 'png') {
        const bitmap = await createImageBitmap(new Blob([bytes(frame.data)], { type: 'image/png' })); bitmaps.push(bitmap);
        if (bitmap.width !== frame.width || bitmap.height !== frame.height) throw new Error('MD-DISPLAY: image dimensions');
        context.drawImage(bitmap, 0, 0);
      } else if (frame.kind === 'video' && videoCodecs.includes(frame.codec) && typeof frame.key === 'boolean') {
        if (globalThis.Worker) {
          this.prepareVideo(frame);
        } else if (!this.decoder || frame.key) {
          if (this.decoder?.state !== 'closed') this.decoder?.close();
          this.decoder = new VideoDecoder({ output: value => (this.decoded ? this.decoded.resolve(value) : value.close()), error: error => this.decoded?.reject(error) });
          this.decoder.configure({ codec:frame.codec, codedWidth:frame.width, codedHeight:frame.height, optimizeForLatency:true });
          this.videoSequence = null;
        }
        if (!frame.key && (this.videoSequence === null || this.sequence !== frame.sequence-1 || this.documentId !== frame.document_id)) throw Error('MD-DISPLAY: video base lost; subscribe afresh');
        let output, timer;
        try {
          output = globalThis.Worker ? await this.prepared.get(frame) : await new Promise((resolve,reject) => {
            timer=setTimeout(()=>reject(Error('MD-DISPLAY: decode timeout')),5000);
            this.decoded={resolve,reject};
            this.decoder.decode(new EncodedVideoChunk({type:frame.key?'key':'delta',timestamp:frame.sequence*33333,data:bytes(frame.data)}));
          });
          if (!((output.displayWidth === frame.width && output.displayHeight === frame.height) || (output.displayWidth === frame.css_width && output.displayHeight === frame.css_height) || (frame.css_width===1920 && frame.css_height===1080 && output.displayWidth===1280 && output.displayHeight===720))) throw Error('MD-DISPLAY: decoded geometry');
          videoFrame=output;output=null;this.videoSequence=frame.sequence;
        } finally { clearTimeout(timer); this.decoded=null; output?.close(); }
      } else throw new Error('MD-DISPLAY: unsupported frame');
      if (this.closed) return false;
      if(!globalThis.Worker||!videoFrame)this.onTiming('client_decode', at);
      // Every accepted sequence is a possible base for the next tile packet,
      // even if a newer video is queued or arrives after this decode finishes.
      // Keep actual canvas pixels aligned with the logical recovery cursor.
      this.didDraw=this.outcomes.get(frame)?.drawn??true;
      this.prediction?.restore();
      const presented = performance.timeOrigin + performance.now();
      if(patch)this.videoSink.overlay(frame);
      if (stripes){
        if(this.canvas.width!==frame.width)this.canvas.width=frame.width;
        if(this.canvas.height!==frame.height)this.canvas.height=frame.height;
        this.stripeDecoder.commit(stripes,this.canvas.getContext('2d'));
      }else if (patch) {
        // All patches are decoded and validated before this synchronous commit.
        const front = this.canvas.getContext('2d');
        if (frame.moves) {
          // Moves read one snapshot of the previous canvas, never a partial result.
          if (!this.scratch || this.scratch.width !== frame.width || this.scratch.height !== frame.height) this.scratch = new OffscreenCanvas(frame.width, frame.height);
          const scratch = this.scratch.getContext('2d');
          scratch.globalCompositeOperation = 'copy';this.videoSink.snapshot(scratch);
          front.imageSmoothingEnabled = false;
          for (const [x, y, w, h, dy] of frame.moves) front.drawImage(this.scratch, x, y - dy, w, h, x, y, w, h);
        }
        for (let i = 0; i < bitmaps.length; i++) front.drawImage(bitmaps[i], frame.tiles[i].x, frame.tiles[i].y);
      } else {
        if(this.canvas.width !== frame.width)this.canvas.width=frame.width;
        if(this.canvas.height !== frame.height)this.canvas.height=frame.height;
        if(videoFrame&&(!globalThis.Worker||this.deferredVideo.has(frame))){
          const deferred=this.deferredVideo.get(frame);this.deferredVideo.delete(frame);
          const outcome=deferred?await this.commitVideo({image:videoFrame,...deferred},frame):await this.videoSink.commit(videoFrame,frame);
          this.outcomes.set(frame,outcome);this.didDraw=outcome.drawn;
        }
        else if(!videoFrame){this.videoSink.overlay(frame);this.canvas.getContext('2d').drawImage(back, 0, 0);}
      }
      this.sequence = frame.sequence; this.documentId = frame.document_id;
      if(!videoFrame){this.decodeSequence=frame.sequence;this.decodeDocumentId=frame.document_id;}
      if(!globalThis.Worker||!videoFrame)this.onTiming('client_present', presented);
      return true;
    } finally { for(const row of stripes??[])row.output.close();videoFrame?.close();for(const bitmap of bitmaps)bitmap?.close(); this.busy = false; }
  }
  input(input) {
    if (this.closed || !this.documentId) throw new Error('MD-DISPLAY: no displayed document');
    if(this.binding.source?.kind==='desktop') {
      const source=this.binding.source,dpr=this.binding.device_scale_factor;
      if(this.documentId!==source.generation)throw Error('MP-11: stale desktop document');
      const submitted={...input};
      if(['click','move','scroll'].includes(input.kind)){
        if(!Number.isFinite(input.x)||!Number.isFinite(input.y)||input.x<0||input.y<0||input.x>=source.width/dpr||input.y>=source.height/dpr)throw Error('MP-11: desktop input geometry');
        submitted.x=Math.floor(input.x*dpr);submitted.y=Math.floor(input.y*dpr);
        if(input.kind==='click')submitted.button=input.button??1;
        if(input.kind==='scroll'){
          if(!Number.isFinite(input.delta_y)||input.delta_x!==0)throw Error('MP-08: desktop wheel unsupported');
          submitted.steps=Math.max(-20,Math.min(20,Math.sign(input.delta_y)*Math.ceil(Math.abs(input.delta_y)/120)));
          delete submitted.delta_x;delete submitted.delta_y;
        }
      }else if(!['key','text','composition'].includes(input.kind))throw Error('MP-08: desktop input unsupported');
      return {op:'computer',command:{op:'input',target:{surface_id:source.surface_id,generation:source.generation},input:submitted}};
    }
    return { op: 'display_input', tab_id: this.binding.tab_id, generation: this.binding.generation, document_id: this.documentId, ...regionInputBinding(this.binding), input };
  }
  // Resources are released once; a preserved paint is cleared by a later close, a cleared canvas is never reset again.
  close({preserveFrame=false}={}) { if(!this.closed){this.closed=true;this.videoSink.close({preserveFrame});this.stripeDecoder?.close();this.prediction?.close();for(const key of ['back','scratch'])if(this[key]){this[key].width=1;this[key].height=1;this[key]=null;} this.workerDecoder?.close();this.workerDecoder=null;if(this.decoder?.state!=='closed')this.decoder?.close(); this.decoder=null; this.documentId = null;} if(!preserveFrame&&!this.cleared){this.cleared=true;this.canvas.width = 1; this.canvas.height = 1;} }
}
// Bounded credit window; events and responses can arrive in either order.
export async function attachBrowserDisplay(canvas, transport, tab, options = {}) {
  // MP-08/MP-10/MP-11: the app adapter must decode authenticated CXD1
  // events before enabling this experimental display. Refuse before opening
  // a stream so an old JSON-only adapter cannot receive unusable pixels.
  if (!Number.isSafeInteger(transport.kernelProtocolVersion) || transport.kernelProtocolVersion < (options.desktop?desktopMinimumProtocolVersion:minimumProtocolVersion) || transport.displayEventEncoding !== 'CXD1')
    throw Error('MP-08/MP-10: protocol466 binary display adapter required');
  if (options.target && transport.kernelProtocolVersion < regionMinimumProtocolVersion)
    throw Error('MP-08/MP-10: protocol519 region display adapter required');
  const request = async command => {
    const reply = await transport.request({ KernelBrowser: { command } });
    return reply.KernelBrowser.result;
  };
  // MP-10: the owner GPU comparison can negotiate the existing whole-video
  // capability. Default stripe negotiation and serialized shapes are unchanged.
  const codecs=options.codec ? [options.codec,'png','chariox-video-dependencies-v1',...(['avc1.420033','vp8'].includes(options.codec)?['chariox-stripes-v1']:[])] : await supportedCodecs();
  if (transport.relayProtocolVersion >= 96) {
    // Keep the existing eight-entry offer bound, retaining the preferred codec.
    if (codecs.length === 8) codecs.splice(codecs.indexOf('png') - 1, 1);
    codecs.push('chariox-relay-binary-v96');
  }
  // MP-08/MP-10: protocol 491 kernels push frames on a relay display
  // subscription; older kernels and other transports keep credits.
  const pushMode = options.push !== false && transport.kernelProtocolVersion >= pushProtocolVersion && typeof transport.subscribeDisplay === 'function';
  const offer={codecs:options.stripes===false?codecs.filter(c=>c!=='chariox-stripes-v1'):codecs,bitrate:options.bitrate??2_000_000,device_scale_factor:options.deviceScaleFactor??1};
  const targetKey=JSON.stringify([options.desktop?tab.surface_id:tab.tab_id,tab.generation,...(options.target?[options.target]:[])]);
  const hint=options.resumeHint;
  const resume=transport.kernelProtocolVersion>=resumeProtocolVersion&&hint?.targetKey===targetKey&&
    hint.desktop===Boolean(options.desktop)&&hint.device_scale_factor===offer.device_scale_factor&&hint.expiresAt>performance.now();
  // MP-08/MP-10/MP-11: start code-only worker loading while the existing
  // authenticated registration is pending. No pixels or binding enter it yet.
  let warmDecoder;
  if(globalThis.Worker&&globalThis.VideoDecoder){try{warmDecoder=new WorkerVideoDecoder();}catch{/* Normal presenter admission handles unavailable codecs. */}}
  let subscribed;
  try{subscribed=await request(resume?{op:'display_resume',subscription_id:hint.subscription_id,generation:hint.generation}
    :options.desktop?{op:'computer',command:{op:'display_subscribe',target:tab,...offer}}:{op:'display_subscribe',...tab,...(options.target?{target:options.target}:{}),...offer,
      ...(pushMode&&transport.initialPush===true&&transport.kernelProtocolVersion>=initialPushProtocolVersion?{initial_push:true}:{}),
      ...(pushMode&&options.tabDisplay===true&&!options.target&&transport.kernelProtocolVersion>=tabDisplayMinimumProtocolVersion?{tab_display:true}:{})});}
  catch(error){warmDecoder?.close();options.onResumeHint?.(null);throw error;}
  // The reply's generation wins: the kernel may restart its browser at the
  // viewer's device scale (protocol 491), rotating the generation.
  const binding=options.desktop?{...subscribed,tab_id:subscribed.source?.surface_id}:{...tab,...subscribed};
  options.onResumeHint?.(null);
  const releaseDisplay=async preserveFrame=>{
    const suspend=preserveFrame&&transport.kernelProtocolVersion>=resumeProtocolVersion&&typeof options.onResumeHint==='function';
    options.onResumeHint?.(suspend?{subscription_id:binding.subscription_id,generation:binding.generation,
      targetKey,desktop:Boolean(options.desktop),device_scale_factor:offer.device_scale_factor,expiresAt:performance.now()+5000}:null);
    try{await request({op:suspend?'display_suspend':'unsubscribe',subscription_id:binding.subscription_id,generation:binding.generation});}
    finally{await transport.unsubscribeDisplay?.(binding);}
  };
  if(options.desktop&&(binding.source?.kind!=='desktop'||binding.source.surface_id!==tab.surface_id||binding.source.generation!==tab.generation||![binding.source.width,binding.source.height].every(n=>Number.isSafeInteger(n)&&n>0&&n<=4096)||binding.device_scale_factor!==offer.device_scale_factor)){
    warmDecoder?.close();
    await request({op:'unsubscribe',subscription_id:binding.subscription_id,generation:binding.generation}).catch(()=>{});
    throw Error('MP-11: invalid desktop subscription binding');
  }
  const actorCommand=op=>op==='actors'?(options.desktop?{op:'computer',command:{op}}:{op:'display_actors'}):options.desktop?{op:'computer',command:{op,target:tab}}:{op:`display_${op}`,tab_id:binding.tab_id,generation:binding.generation};
  // MP-08/MP-10/MP-11: a default page crop is activated only at allocated536.
  // Explicit519 regions and pre536 full-viewport producers keep their contracts.
  if(!options.desktop&&!options.target&&binding.source_kind==='region'&&transport.kernelProtocolVersion<visiblePageMinimumProtocolVersion){
    warmDecoder?.close();
    await releaseDisplay(false).catch(()=>{});throw Error('MP-08/MP-11: protocol536 visible page adapter required');
  }
  if((options.target||binding.source_kind==='region')&&!regionBindingCurrent(binding)){
    warmDecoder?.close();
    await releaseDisplay(false).catch(()=>{});throw Error('MP-08/MP-11: invalid region negotiation');
  }
  const onTiming = options.onTiming ?? (() => {});
  // MP-08/MP-10/MP-11: crops and Chrome UI paint into their actual canvas.
  // Avoid an intermediate media-track compositor and another rAF for regions.
  const presenter = new BrowserDisplayPresenter(canvas, binding, onTiming, warmDecoder,
    {direct:!options.desktop&&(binding.source_kind==='region'||['region','browser_chrome','browser_auxiliary'].includes(options.target?.kind)),
     // The Chrome UI band keeps its immediate draw (keyboard echo); only the page crop is paced.
     paced:!options.desktop&&(options.target?options.target.kind==='region':binding.source_kind==='region')});
  if(options.scrollPredictionRegion)presenter.prediction=new ScrollPrediction(canvas,options.scrollPredictionRegion,context=>presenter.videoSink.snapshot(context));
  let pending = null, stopped = false, creditOutstanding = false, predictionEpoch=0;
  let running = false, failure = null, active = new Set();
  const presentationQueue=new PresentationQueue(presenter);
  const frames = [], arrivals = [];
  let nextSequence=null,heldBytes=0;
  const held=new Map();
  let queuedBytes = 0;
  const accept = frame => {
    const size = frameBytes(frame);
    if (frames.length >= 8 || queuedBytes + size > 1024 * 1024) throw Error('MD-DISPLAY: credited receive window exceeded');
    const presented = presentationQueue.submit(frame,async () => {
      if (!await presenter.present(frame)) throw Error('MD-DISPLAY: rejected window frame');
      // MP-08/MP-10: accepted hidden pixels still define input/capture geometry.
      // Keep this separate from visible presentation metrics.
      options.onAccepted?.(frame);
      if(presenter.wasDrawn(frame) !== false) options.onPresented?.(frame,presenter.outcomes.get(frame));
    });
    presented.catch(error => { failure=error; running=false; });
    const item={frame,presented,size};
    if (arrivals.length) arrivals.shift()(item);
    else { frames.push(item); queuedBytes += size; }
  };
  const closedItem = { presented: Promise.resolve() };
  const receive = () => frames.length ? Promise.resolve((queuedBytes -= frames[0].size, frames.shift())) : stopped ? Promise.resolve(closedItem) : new Promise(resolve => arrivals.push(resolve));
  const initialEvents=[];let initialBytes=0,initialLost=false;
  const off = transport.onEvent(event => {
    if(pushMode&&event.event==='kernel_browser_frame'&&event.subscription_id===binding.subscription_id){
      const size=frameBytes(event.frame);
      if(initialEvents.length>=8||initialBytes+size>8*1024*1024){
        initialEvents.length=0;initialBytes=0;initialLost=true;
      }
      // Registration can flush a burst held by the relay. Keep the bounded window and
      // recover from an independent frame just as the running stream does after a gap.
      if(initialLost&&!independentFrame(event.frame)||size>8*1024*1024)return;
      initialLost=false;initialBytes+=size;initialEvents.push(event);return;
    }
    if (event.event !== 'kernel_browser_frame' || event.subscription_id !== binding.subscription_id) return;
    if (running || active.size) {
      try {
        const frame=event.frame,size=frameBytes(frame);
        if(!Number.isSafeInteger(frame.sequence)||frame.sequence<nextSequence||held.has(frame.sequence)||held.size+frames.length>=8||heldBytes+queuedBytes+size>1024*1024) throw Error('MD-DISPLAY: reordered receive window exceeded');
        held.set(frame.sequence,frame);heldBytes+=size;
        // Relay control/large-event lanes and concurrent decryption may finish
        // in different orders. Decode every dependency in source sequence.
        while(held.has(nextSequence)) {
          const ordered=held.get(nextSequence);held.delete(nextSequence++);
          heldBytes-=frameBytes(ordered);accept(ordered);
        }
      } catch (error) { failure = error; running = false; }
    } else { pending?.resolve(event.frame); pending = null; }
  });
  try { await transport.subscribeDisplay?.(binding); }
  catch (error) {
    off(); presenter.close({preserveFrame:true});
    await releaseDisplay(true).catch(() => {});
    throw error;
  }
  if (pushMode) {
    const display=pushDisplay(presenter, transport, request, binding, actorCommand, {...options,releaseDisplay,initialEvents,initialLost});
    off();return display;
  }
  const next = async () => {
    if (stopped || creditOutstanding || running || active.size) throw new Error('MD-DISPLAY: stream stopped or credit outstanding');
    creditOutstanding = true;
    let timer;
    const event = new Promise((resolve, reject) => {
      timer = setTimeout(() => { pending = null; reject(new Error('MD-DISPLAY: event timeout')); }, 30000);
      pending = { resolve, reject };
    });
    // Attach rejection immediately even when request fails first.
    event.catch(() => {});
    const presented = event.then(async frame => {
      if (!frame) return null;
      if (!await presenter.present(frame)) throw new Error('MD-DISPLAY: rejected frame');
      options.onAccepted?.(frame);
      if(presenter.wasDrawn(frame)!==false) options.onPresented?.(frame,presenter.outcomes.get(frame));
      return frame;
    });
    presented.catch(() => {});
    try {
      const at = performance.timeOrigin + performance.now();
      const receipt = await request({ op: 'display_next', subscription_id: binding.subscription_id, generation: binding.generation, after_sequence: presenter.sequence });
      onTiming('frame_credit_round_trip', at);
      // No changed pixels is represented by no event; result stays null.
      // Request receipts identify whether a frame was sent.
      if (!receipt.frame_sent) { pending?.resolve(null); return null; }
      const waitAt = performance.timeOrigin + performance.now();
      const frame = await presented;
      onTiming('client_event_wait', waitAt);
      return frame;
    } finally { clearTimeout(timer); pending = null; creditOutstanding = false; }
  };
  // MD-DISPLAY-04: four admitted requests pipeline the existing 419 credit
  // operation. Each slot is retained through its receipt AND presentation;
  // source/encoder stay serial, so no unbounded capture queue is introduced.
  let receiptOrder = Promise.resolve();
  const idle=new IdleCredit();
  const issue = () => {
    const at = performance.timeOrigin + performance.now();
    const response = request({ op:'display_next', subscription_id:binding.subscription_id,
      generation:binding.generation, after_sequence:presenter.sequence });
    response.catch(() => {});
    const ordered = receiptOrder.then(async () => {
      const receipt = await response;
      idle.observeRoundTrip(performance.timeOrigin+performance.now()-at);
      onTiming('frame_credit_round_trip', at);
      if (!receipt.frame_sent) return false;
      idle.wake();
      let timer;
      const item = await Promise.race([receive(), new Promise((_,reject) => {
        timer=setTimeout(() => reject(Error('MD-DISPLAY: window event timeout')),30000);
      })]).finally(() => clearTimeout(timer));
      await item.presented;
      return true;
    });
    receiptOrder = ordered.catch(() => {});
    const slot=ordered.then(changed=>{if(!changed&&running)return idle.wait();});
    active.add(slot);
    slot.catch(error => { failure=error; running=false; }).finally(() => {
      active.delete(slot);
      if (running && !failure) issue();
    });
  };
  const start = () => {
    if (stopped || creditOutstanding || active.size || failure) throw failure ?? Error('MD-DISPLAY: stream busy');
    running=true;nextSequence=presenter.sequence+1;
    const desired=options.creditWindow ?? 3;
    if(!Number.isSafeInteger(desired)||desired<1||desired>8) {running=false;throw Error('MD-DISPLAY: invalid credit window');}
    const batch=Math.min(192000,Math.max(24000,(binding.bitrate??options.bitrate??2000000)/8*.5*.75-4096));
    const count=Math.min(desired,Math.max(1,Math.floor(1024*1024/(batch*4/3+4096))));
    for(let i=0;i<count;i++) issue();
  };
  const stop = async () => { running=false;idle.wake(); await Promise.allSettled([...active]); if(failure) throw failure; };
  return { binding, presenter, next, start, stop,
    get running() { return running; },
    get error() { return failure; },
    input: async input => {presenter.prediction?.restore();idle.wake();const submitted={...input},sequence=presenter.sequence,epoch=++predictionEpoch;const reply=await request(presenter.input(submitted));if(!stopped){idle.wake();if(epoch===predictionEpoch&&presenter.sequence===sequence)presenter.prediction?.predict(submitted,options.deviceScaleFactor??1)}return reply;},
    takeover: () => {predictionEpoch++;presenter.prediction?.restore();return request(actorCommand('takeover'));},
    release: () => {predictionEpoch++;presenter.prediction?.restore();return request(actorCommand('release'));},
    actors: () => request(actorCommand('actors')),
    async close(options) {
      // MP-11: detach first; parked credits cannot hold teardown for their window timeout.
      running = false; stopped = true; pending?.reject(new Error('MD-DISPLAY: closed')); pending = null; off(); presenter.close(options);
      for (const arrive of arrivals.splice(0)) arrive(closedItem);
      await releaseDisplay(options?.preserveFrame);
      await stop().catch(() => {});
    },
  };
}

// MP-08/MP-10: protocol 491 pushed display. The kernel pump sends frames as
// they are produced; the viewer acknowledges each presented sequence (plus a
// heartbeat), which drives the kernel's ACK gate (after Selkies' frame ACK).
function pushDisplay(presenter, transport, request, binding, actorCommand, options) {
  const {onTiming = () => {}, idleMs = 400, heartbeatMs = 1000, gapMs = Math.min(250, heartbeatMs)} = options;
  const held = new Map(), unread = [], waiters = new Set();
  const presentationQueue=new PresentationQueue(presenter,frame=>!recovering||independent(frame));
  let nextSequence = presenter.sequence + 1, heldBytes = 0, failure = null, running = false, closed = false, ackedAt = 0, predictionEpoch = 0;
  let recovering = false, keyRequestedAt = -Infinity, gapTimer = null, paused = options.watched?.value === false, resync = false;
  const fail = error => { failure ??= error; for (const wake of waiters) wake(); };
  // MP-08/MP-10/MP-11: ACK replies must not delay delivery feedback by another
  // WAN round trip. Bound outstanding requests and coalesce only at that bound.
  const document=presenter.canvas.ownerDocument;
  let ackInFlight=0, queuedAck=null, timedAcks=false;
  const receiptTimes=new Map();
  const ack = (sequence, lost = false) => {
    if (closed) return;
    const candidate={sequence:Math.max(sequence,queuedAck?.sequence??sequence),lost:lost||queuedAck?.lost||false};
    if (ackInFlight>=8) { queuedAck=candidate;return; }
    queuedAck=null;ackInFlight++;ackedAt=performance.now();
    const received=receiptTimes.get(candidate.sequence);
    if(timedAcks&&received!==undefined)candidate.held_ms=Math.min(60000,Math.max(0,Math.floor(performance.timeOrigin+performance.now()-received)));
    for(const sequence of receiptTimes.keys())if(sequence<=candidate.sequence)receiptTimes.delete(sequence);
    request({ op: 'display_ack', subscription_id: binding.subscription_id, generation: binding.generation, ...candidate })
      .then(reply => { if (reply?.push !== 'running') fail(Error('MD-DISPLAY: push pump ' + (reply?.push ?? 'unavailable'))); else if(transport.kernelProtocolVersion>=536&&reply.ack_timing===true)timedAcks=true; }, fail)
      .finally(()=>{
        ackInFlight--;
        if(!closed&&!failure&&queuedAck){const latest=queuedAck;queuedAck=null;ack(latest.sequence,latest.lost);}
      });
  };
  const visibilityChanged=()=>{if(!document.hidden&&!paused)ack(presenter.sequence,queuedAck?.lost??false)};
  document?.addEventListener('visibilitychange',visibilityChanged);
  // MP-08/MP-10 (1.5): a lost base, decode failure or receive gap keeps the
  // canvas, asks for an independent frame and drops dependent frames until
  // it arrives. Only a binding/geometry mismatch or a stopped pump is fatal.
  const independent = independentFrame;
  const recover = () => {
    recovering = true;
    if (performance.now() - keyRequestedAt >= 1000) { keyRequestedAt = performance.now(); ack(presenter.sequence, true); }
  };
  const present = frame => {
    presentationQueue.submit(frame,async () => {
      if (failure || closed) return;
      if (recovering && !independent(frame)) { ack(frame.sequence); return recover(); }
      const at = performance.timeOrigin + performance.now();
      try { if (!await presenter.present(frame)) return; }
      catch (error) {
        if (/invalid geometry\/binding/.test(error?.message ?? '')) throw error;
        onTiming('client_recover_request', at); ack(frame.sequence); return recover();
      }
      recovering = false;
      onTiming('client_present_total', at);
      options.onAccepted?.(frame);
      if (presenter.wasDrawn(frame) !== false) options.onPresented?.(frame,presenter.outcomes.get(frame));
      ack(frame.sequence);
      unread.push(frame); if (unread.length > 64) unread.shift();
      for (const wake of waiters) wake();
    }).catch(fail);
  };
  // Skip a receive gap to `to`, dropping older held frames, and recover from a key.
  const skip = to => {
    for (const [sequence, frame] of held) if (sequence < to) { held.delete(sequence); heldBytes -= frameBytes(frame); }
    nextSequence = to; recover();
  };
  const drain = () => {
    while (held.has(nextSequence)) { const ordered = held.get(nextSequence); held.delete(nextSequence++); heldBytes -= frameBytes(ordered); present(ordered); }
    clearTimeout(gapTimer); gapTimer = null;
    // MP-08/MP-10: a missing sequence waits a bounded reorder window, then
    // the presenter asks for an independent frame and skips it (the kernel
    // ACK gate may stop sending long before a queue overflow).
    if (held.size) gapTimer = setTimeout(() => { gapTimer = null; if (!closed && held.size && !held.has(nextSequence)) { skip(Math.min(...held.keys())); drain(); } }, gapMs);
  };
  const receiveEvent = event => {
    if (event.event !== 'kernel_browser_frame' || event.subscription_id !== binding.subscription_id || failure || closed || paused) return;
    const frame = event.frame, size = frameBytes(frame);
    if (resync && Number.isSafeInteger(frame.sequence)) { resync = false; nextSequence = frame.sequence; }
    // Relay lanes and concurrent decryption may reorder; decode in sequence.
    if (!Number.isSafeInteger(frame.sequence) || frame.sequence < nextSequence || held.has(frame.sequence)) return;
    const received=frame[Symbol.for('chariox.display.receivedAt')];
    receiptTimes.set(frame.sequence,Number.isFinite(received)?received:performance.timeOrigin+performance.now());
    while(receiptTimes.size>64)receiptTimes.delete(receiptTimes.keys().next().value);
    held.set(frame.sequence, frame); heldBytes += size;
    // A sequence that never arrives: skip the gap and recover from a key.
    // While recovering, an independent frame establishes the cursor at once.
    if (held.size > 8 || heldBytes > 8 * 1024 * 1024) skip(Math.min(...held.keys()));
    else if (frame.sequence > nextSequence && independent(frame)) skip(frame.sequence);
    drain();
  };
  const off=transport.onEvent(receiveEvent);
  if(options.initialLost)recover();
  for(const event of options.initialEvents??[])receiveEvent(event);
  if(options.initialEvents)options.initialEvents.length=0;
  // A stopped stream sends no heartbeats: the kernel ACK gate stops its pump.
  // While recovering, the heartbeat repeats the independent-frame request: it also lifts a kernel pause whose resume ACK was overtaken.
  const heartbeat = setInterval(() => { if (!paused && performance.now() - ackedAt >= heartbeatMs) ack(presenter.sequence, recovering); }, heartbeatMs);
  // A stream runs only while a person can see it. Unwatched, the kernel pump is told to pause (no capture, no frames, no
  // reprobe) and a sparse paused ACK keeps the delivery lease alive; no frames are kept here. Being watched again asks for one
  // independent frame, which also lifts the pause.
  let pauseTimer = null;
  const pauseKernel = () => {
    if (closed || failure) return;
    request({ op: 'display_ack', subscription_id: binding.subscription_id, generation: binding.generation, sequence: presenter.sequence, lost: false, paused: true })
      .then(reply => { if (reply?.push !== 'running') fail(Error('MD-DISPLAY: push pump ' + (reply?.push ?? 'unavailable'))); }, fail);
  };
  const halt = () => {
    paused = true; clearTimeout(gapTimer); gapTimer = null; held.clear(); heldBytes = 0; queuedAck = null;
    if (pauseTimer === null) { pauseKernel(); pauseTimer = setInterval(pauseKernel, options.pauseKeepaliveMs ?? 20_000); }
  };
  const resume = () => { if (paused && options.watched?.value !== false) { paused = false; clearInterval(pauseTimer); pauseTimer = null; resync = true; keyRequestedAt = -Infinity; recover(); } };
  const unwatch = options.watched?.subscribe(watched => { if (watched) { if (running) resume(); } else halt(); });
  if (paused) halt(); else ack(presenter.sequence);
  const next = async () => {
    if (failure) throw failure;
    if (!unread.length) await new Promise(resolve => { const wake = () => { clearTimeout(timer); waiters.delete(wake); resolve(); }; const timer = setTimeout(wake, idleMs); waiters.add(wake); });
    if (failure) throw failure;
    return unread.shift() ?? null;
  };
  return { binding, presenter, next, push: true,
    // MP-08/MP-10: stop pauses reception, presentation and acknowledgements;
    // start resumes from an independent frame (frames were dropped meanwhile).
    start: () => { if (failure) throw failure; running = true; unread.length = 0; resume(); },
    stop: async () => { running = false; halt(); await presentationQueue.tail; if (failure) throw failure; },
    get running() { return running; },
    get error() { return failure; },
    requestKey: () => ack(presenter.sequence, true),
    // P1 (protocol 720): a Tab display sends nothing while parked; the Tab's next commit unparks it in the kernel,
    // so its first frame arrives without a request. Reception keeps running.
    park: parked => binding.tab_display === true ? request({ op: 'display_park', subscription_id: binding.subscription_id, generation: binding.generation, parked }) : Promise.reject(Error('MP-08: not a Tab display')),
    input: async input => {presenter.prediction?.restore();const submitted={...input},sequence=presenter.sequence,epoch=++predictionEpoch;const reply=await request(presenter.input(submitted));if(!closed&&epoch===predictionEpoch&&presenter.sequence===sequence)presenter.prediction?.predict(submitted,options.deviceScaleFactor??1);return reply;},
    takeover: () => {predictionEpoch++;presenter.prediction?.restore();return request(actorCommand('takeover'));},
    release: () => {predictionEpoch++;presenter.prediction?.restore();return request(actorCommand('release'));},
    actors: () => request(actorCommand('actors')),
    async close(closeOptions) { closed = true; running = false; queuedAck=null;receiptTimes.clear();document?.removeEventListener('visibilitychange',visibilityChanged); clearInterval(heartbeat); clearInterval(pauseTimer); clearTimeout(gapTimer); unwatch?.(); off(); presentationQueue.cancel(); presenter.close(closeOptions); await presentationQueue.tail.catch(() => {}); await options.releaseDisplay(closeOptions?.preserveFrame); },
  };
}
