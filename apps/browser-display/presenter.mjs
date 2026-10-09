import {StripePresenter,independentStripeCover} from './stripe-presenter.mjs';
import {ScrollPrediction} from './scroll-prediction.mjs';
import {WorkerVideoDecoder} from './decoder-worker.mjs';
// MD-DISPLAY-04: protocol 419 presentation only. Cloud supplies its existing
// admitted/encrypted kernel request and event adapter; never a Cloud media proxy.
export const minimumProtocolVersion = 466;
export const desktopMinimumProtocolVersion = 474;
export const pushProtocolVersion = 475;
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
// MP-08/MP-11: desktop geometry comes from the kernel-owned surface binding.
function validGeometry(frame,binding) {
  const source=binding.source;
  if(source?.kind==='desktop')return frame.document_id===source.generation&&frame.device_scale_factor===binding.device_scale_factor&&frame.width===source.width&&frame.height===source.height&&frame.css_width===source.width/frame.device_scale_factor&&frame.css_height===source.height/frame.device_scale_factor;
  return (frame.css_width===1280&&frame.css_height===800)||(frame.css_width===1920&&frame.css_height===1080&&frame.device_scale_factor===1);
}
export class BrowserDisplayPresenter {
  constructor(canvas, binding, onTiming = () => {}) {
    this.canvas = canvas; this.binding = binding; this.sequence = 0; this.documentId = null;
    this.busy = false; this.closed = false;
    this.onTiming = onTiming;
  }
  async present(frame) {
    if (this.closed) return false;
    if (this.busy) throw new Error('MD-DISPLAY: await presentation before granting next credit');
    if (frame.subscription_id !== this.binding.subscription_id || frame.generation !== this.binding.generation || frame.tab_id !== this.binding.tab_id || frame.sequence <= this.sequence) return false;
    if (!Number.isSafeInteger(frame.sequence) || ![1, 2].includes(frame.device_scale_factor) || frame.width !== frame.css_width * frame.device_scale_factor || frame.height !== frame.css_height * frame.device_scale_factor || !validGeometry(frame,this.binding) || typeof frame.document_id !== 'string' || !frame.document_id || frame.document_id.length > 256) throw new Error('MD-DISPLAY: invalid geometry/binding');
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
        this.stripeDecoder??=new StripePresenter();
        stripes=await this.stripeDecoder.decode(frame,bytes);
      }else if (frame.kind === 'tiles') {
        if (!Array.isArray(frame.tiles) || frame.tiles.length > 260) throw new Error('MD-DISPLAY: tile count');
        for (const tile of frame.tiles) {
          // MP-08/MP-10: lossless PNG tiles or WebP strips up to 2560x256 pixels.
          if (![tile.x, tile.y, tile.width, tile.height].every(Number.isSafeInteger) || tile.x < 0 || tile.y < 0 || tile.width < 1 || tile.height < 1 || tile.width * tile.height > 2560 * 256 || tile.x + tile.width > frame.width || tile.y + tile.height > frame.height || !Object.hasOwn(imageTypes, tile.format)) throw new Error('MD-DISPLAY: tile geometry');
          const bitmap = await createImageBitmap(new Blob([bytes(tile.data)], { type: imageTypes[tile.format] })); bitmaps.push(bitmap);
          if (bitmap.width !== tile.width || bitmap.height !== tile.height) throw new Error('MD-DISPLAY: tile dimensions');
        }
      } else if (frame.kind === 'png') {
        const bitmap = await createImageBitmap(new Blob([bytes(frame.data)], { type: 'image/png' })); bitmaps.push(bitmap);
        if (bitmap.width !== frame.width || bitmap.height !== frame.height) throw new Error('MD-DISPLAY: image dimensions');
        context.drawImage(bitmap, 0, 0);
      } else if (frame.kind === 'video' && videoCodecs.includes(frame.codec) && typeof frame.key === 'boolean') {
        if (globalThis.Worker) {
          // A timed-out or crashed worker closes itself; a key starts afresh.
          if(!this.workerDecoder||this.workerDecoder.closed)this.workerDecoder=new WorkerVideoDecoder();
        } else if (!this.decoder || frame.key) {
          if (this.decoder?.state !== 'closed') this.decoder?.close();
          this.decoder = new VideoDecoder({ output: value => (this.decoded ? this.decoded.resolve(value) : value.close()), error: error => this.decoded?.reject(error) });
          this.decoder.configure({ codec:frame.codec, codedWidth:frame.width, codedHeight:frame.height, optimizeForLatency:true });
          this.videoSequence = null;
        }
        if (!frame.key && (this.videoSequence === null || this.sequence !== frame.sequence-1 || this.documentId !== frame.document_id)) throw Error('MD-DISPLAY: video base lost; subscribe afresh');
        let output, timer;
        try {
          output = this.workerDecoder ? await this.workerDecoder.decode(frame,bytes(frame.data)) : await new Promise((resolve,reject) => {
            timer=setTimeout(()=>reject(Error('MD-DISPLAY: decode timeout')),5000);
            this.decoded={resolve,reject};
            this.decoder.decode(new EncodedVideoChunk({type:frame.key?'key':'delta',timestamp:frame.sequence*33333,data:bytes(frame.data)}));
          });
          if (!((output.displayWidth === frame.width && output.displayHeight === frame.height) || (output.displayWidth === frame.css_width && output.displayHeight === frame.css_height) || (frame.css_width===1920 && frame.css_height===1080 && output.displayWidth===1280 && output.displayHeight===720))) throw Error('MD-DISPLAY: decoded geometry');
          videoFrame=output;output=null;this.videoSequence=frame.sequence;
        } finally { clearTimeout(timer); this.decoded=null; output?.close(); }
      } else throw new Error('MD-DISPLAY: unsupported frame');
      if (this.closed) return false;
      this.onTiming('client_decode', at);
      // Every accepted sequence is a possible base for the next tile packet,
      // even if a newer video is queued or arrives after this decode finishes.
      // Keep actual canvas pixels aligned with the logical recovery cursor.
      this.didDraw=true;
      this.prediction?.restore();
      const presented = performance.timeOrigin + performance.now();
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
          scratch.globalCompositeOperation = 'copy'; scratch.drawImage(this.canvas, 0, 0);
          front.imageSmoothingEnabled = false;
          for (const [x, y, w, h, dy] of frame.moves) front.drawImage(this.scratch, x, y - dy, w, h, x, y, w, h);
        }
        for (let i = 0; i < bitmaps.length; i++) front.drawImage(bitmaps[i], frame.tiles[i].x, frame.tiles[i].y);
      } else {
        if(this.canvas.width !== frame.width)this.canvas.width=frame.width;
        if(this.canvas.height !== frame.height)this.canvas.height=frame.height;
        if(videoFrame)this.canvas.getContext('2d').drawImage(videoFrame,0,0,frame.width,frame.height);
        else this.canvas.getContext('2d').drawImage(back, 0, 0);
      }
      this.sequence = frame.sequence; this.documentId = frame.document_id;
      this.onTiming('client_present', presented);
      return true;
    } finally { for(const row of stripes??[])row.output.close();videoFrame?.close();for(const bitmap of bitmaps)bitmap.close(); this.busy = false; }
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
    return { op: 'display_input', tab_id: this.binding.tab_id, generation: this.binding.generation, document_id: this.documentId, input };
  }
  close() { this.stripeDecoder?.close();this.prediction?.close();for(const key of ['back','scratch'])if(this[key]){this[key].width=1;this[key].height=1;this[key]=null;} this.workerDecoder?.close();this.workerDecoder=null;if(this.decoder?.state!=='closed')this.decoder?.close(); this.decoder=null; this.closed = true; this.documentId = null; this.canvas.width = 1; this.canvas.height = 1; }
}
// Bounded credit window; events and responses can arrive in either order.
export async function attachBrowserDisplay(canvas, transport, tab, options = {}) {
  // MP-08/MP-10/MP-11: the app adapter must decode authenticated CXD1
  // events before enabling this experimental display. Refuse before opening
  // a stream so an old JSON-only adapter cannot receive unusable pixels.
  if (!Number.isSafeInteger(transport.kernelProtocolVersion) || transport.kernelProtocolVersion < (options.desktop?desktopMinimumProtocolVersion:minimumProtocolVersion) || transport.displayEventEncoding !== 'CXD1')
    throw Error('MP-08/MP-10: protocol466 binary display adapter required');
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
  // MP-08/MP-10: protocol 475 kernels push frames on a relay display
  // subscription; older kernels and other transports keep credits.
  const pushMode = options.push !== false && transport.kernelProtocolVersion >= pushProtocolVersion && typeof transport.subscribeDisplay === 'function';
  const offer={codecs:options.stripes===false?codecs.filter(c=>c!=='chariox-stripes-v1'):codecs,bitrate:options.bitrate??2_000_000,device_scale_factor:options.deviceScaleFactor??1};
  const subscribed=await request(options.desktop?{op:'computer',command:{op:'display_subscribe',target:tab,...offer}}:{op:'display_subscribe',...tab,...offer});
  // The reply's generation wins: the kernel may restart its browser at the
  // viewer's device scale (protocol 475), rotating the generation.
  const binding=options.desktop?{...subscribed,tab_id:subscribed.source?.surface_id}:{...tab,...subscribed};
  if(options.desktop&&(binding.source?.kind!=='desktop'||binding.source.surface_id!==tab.surface_id||binding.source.generation!==tab.generation||![binding.source.width,binding.source.height].every(n=>Number.isSafeInteger(n)&&n>0&&n<=4096)||binding.device_scale_factor!==offer.device_scale_factor)){
    await request({op:'unsubscribe',subscription_id:binding.subscription_id,generation:binding.generation}).catch(()=>{});
    throw Error('MP-11: invalid desktop subscription binding');
  }
  const actorCommand=op=>op==='actors'?(options.desktop?{op:'computer',command:{op}}:{op:'display_actors'}):options.desktop?{op:'computer',command:{op,target:tab}}:{op:`display_${op}`,tab_id:binding.tab_id,generation:binding.generation};
  const onTiming = options.onTiming ?? (() => {});
  const presenter = new BrowserDisplayPresenter(canvas, binding, onTiming);
  if(options.scrollPredictionRegion)presenter.prediction=new ScrollPrediction(canvas,options.scrollPredictionRegion);
  let pending = null, stopped = false, creditOutstanding = false, predictionEpoch=0;
  let running = false, failure = null, presentation = Promise.resolve(), active = new Set();
  const frames = [], arrivals = [];
  let nextSequence=null,heldBytes=0;
  const held=new Map();
  let queuedBytes = 0;
  const accept = frame => {
    const size = frameBytes(frame);
    if (frames.length >= 8 || queuedBytes + size > 1024 * 1024) throw Error('MD-DISPLAY: credited receive window exceeded');
    const presented = presentation.then(async () => {
      if (!await presenter.present(frame)) throw Error('MD-DISPLAY: rejected window frame');
      if(presenter.didDraw !== false) options.onPresented?.(frame);
    });
    presentation=presented;
    presented.catch(error => { failure=error; running=false; });
    const item={frame,presented,size};
    if (arrivals.length) arrivals.shift()(item);
    else { frames.push(item); queuedBytes += size; }
  };
  const closedItem = { presented: Promise.resolve() };
  const receive = () => frames.length ? Promise.resolve((queuedBytes -= frames[0].size, frames.shift())) : stopped ? Promise.resolve(closedItem) : new Promise(resolve => arrivals.push(resolve));
  const off = transport.onEvent(event => {
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
    off(); presenter.close();
    await request({ op: 'unsubscribe', subscription_id: binding.subscription_id, generation: binding.generation }).catch(() => {});
    throw error;
  }
  if (pushMode) { off(); return pushDisplay(presenter, transport, request, binding, actorCommand, options); }
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
      options.onPresented?.(frame);
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
    async close() {
      // MP-11: detach first; parked credits cannot hold teardown for their window timeout.
      running = false; stopped = true; pending?.reject(new Error('MD-DISPLAY: closed')); pending = null; off(); presenter.close();
      for (const arrive of arrivals.splice(0)) arrive(closedItem);
      await transport.unsubscribeDisplay?.(binding); await request({ op: 'unsubscribe', subscription_id: binding.subscription_id, generation: binding.generation });
      await stop().catch(() => {});
    },
  };
}

// MP-08/MP-10: protocol 475 pushed display. The kernel pump sends frames as
// they are produced; the viewer acknowledges each presented sequence (plus a
// heartbeat), which drives the kernel's ACK gate (after Selkies' frame ACK).
function pushDisplay(presenter, transport, request, binding, actorCommand, options) {
  const {onTiming = () => {}, idleMs = 400, heartbeatMs = 1000, gapMs = Math.min(250, heartbeatMs)} = options;
  const held = new Map(), unread = [], waiters = new Set();
  let nextSequence = presenter.sequence + 1, heldBytes = 0, chain = Promise.resolve(), failure = null, running = false, closed = false, ackedAt = 0, predictionEpoch = 0;
  let recovering = false, keyRequestedAt = -Infinity, gapTimer = null, paused = false, resync = false;
  const fail = error => { failure ??= error; for (const wake of waiters) wake(); };
  const ack = (sequence, lost = false) => {
    if (closed) return;
    ackedAt = performance.now();
    request({ op: 'display_ack', subscription_id: binding.subscription_id, generation: binding.generation, sequence, lost })
      .then(reply => { if (reply?.push !== 'running') fail(Error('MD-DISPLAY: push pump ' + (reply?.push ?? 'unavailable'))); }, fail);
  };
  // MP-08/MP-10 (1.5): a lost base, decode failure or receive gap keeps the
  // canvas, asks for an independent frame and drops dependent frames until
  // it arrives. Only a binding/geometry mismatch or a stopped pump is fatal.
  const independent = frame => frame.kind === 'png' || frame.kind === 'video' && frame.key === true || independentStripeCover(frame);
  const recover = () => {
    recovering = true;
    if (performance.now() - keyRequestedAt >= 1000) { keyRequestedAt = performance.now(); ack(presenter.sequence, true); }
  };
  const present = frame => {
    chain = chain.then(async () => {
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
      if (presenter.didDraw !== false) options.onPresented?.(frame);
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
  const off = transport.onEvent(event => {
    if (event.event !== 'kernel_browser_frame' || event.subscription_id !== binding.subscription_id || failure || closed || paused) return;
    const frame = event.frame, size = frameBytes(frame);
    if (resync && Number.isSafeInteger(frame.sequence)) { resync = false; nextSequence = frame.sequence; }
    // Relay lanes and concurrent decryption may reorder; decode in sequence.
    if (!Number.isSafeInteger(frame.sequence) || frame.sequence < nextSequence || held.has(frame.sequence)) return;
    held.set(frame.sequence, frame); heldBytes += size;
    // A sequence that never arrives: skip the gap and recover from a key.
    // While recovering, an independent frame establishes the cursor at once.
    if (held.size > 8 || heldBytes > 8 * 1024 * 1024) skip(Math.min(...held.keys()));
    else if (frame.sequence > nextSequence && independent(frame)) skip(frame.sequence);
    drain();
  });
  // A stopped stream sends no heartbeats: the kernel ACK gate stops its pump.
  const heartbeat = setInterval(() => { if (!paused && performance.now() - ackedAt >= heartbeatMs) ack(presenter.sequence); }, heartbeatMs);
  ack(presenter.sequence);
  const next = async () => {
    if (failure) throw failure;
    if (!unread.length) await new Promise(resolve => { const wake = () => { clearTimeout(timer); waiters.delete(wake); resolve(); }; const timer = setTimeout(wake, idleMs); waiters.add(wake); });
    if (failure) throw failure;
    return unread.shift() ?? null;
  };
  return { binding, presenter, next, push: true,
    // MP-08/MP-10: stop pauses reception, presentation and acknowledgements;
    // start resumes from an independent frame (frames were dropped meanwhile).
    start: () => { if (failure) throw failure; running = true; unread.length = 0; if (paused) { paused = false; resync = true; keyRequestedAt = -Infinity; recover(); } },
    stop: async () => { running = false; paused = true; clearTimeout(gapTimer); gapTimer = null; held.clear(); heldBytes = 0; await chain; if (failure) throw failure; },
    get running() { return running; },
    get error() { return failure; },
    requestKey: () => ack(presenter.sequence, true),
    input: async input => {presenter.prediction?.restore();const submitted={...input},sequence=presenter.sequence,epoch=++predictionEpoch;const reply=await request(presenter.input(submitted));if(!closed&&epoch===predictionEpoch&&presenter.sequence===sequence)presenter.prediction?.predict(submitted,options.deviceScaleFactor??1);return reply;},
    takeover: () => {predictionEpoch++;presenter.prediction?.restore();return request(actorCommand('takeover'));},
    release: () => {predictionEpoch++;presenter.prediction?.restore();return request(actorCommand('release'));},
    actors: () => request(actorCommand('actors')),
    async close() { closed = true; running = false; clearInterval(heartbeat); clearTimeout(gapTimer); off(); await chain.catch(() => {}); presenter.close(); await transport.unsubscribeDisplay?.(binding); await request({ op: 'unsubscribe', subscription_id: binding.subscription_id, generation: binding.generation }); },
  };
}
