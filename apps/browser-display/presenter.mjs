// MD-DISPLAY-04: protocol 419 presentation only. Cloud supplies its existing
// admitted/encrypted kernel request and event adapter; never a Cloud media proxy.
export const minimumProtocolVersion = 419;
const VP9 = 'vp09.00.10.08';
// Empty credits must not saturate a narrow link with control traffic. Admitted
// input and changed frames wake every parked slot without waiting for a timer.
export class IdleCredit {
  constructor(now=()=>performance.now()) { this.delay=0; this.parked=new Set();this.now=now;this.activeUntil=-Infinity; }
  wake() { this.delay=0;this.activeUntil=this.now()+300;for(const wake of this.parked)wake(); }
  wait() {
    const active=this.now()<this.activeUntil;
    this.delay=Math.min(active?33:100,Math.max(active?8:32,this.delay*2));
    return new Promise(resolve=>{
      const wake=()=>{clearTimeout(timer);this.parked.delete(wake);resolve();};
      const timer=setTimeout(wake,this.delay);this.parked.add(wake);
    });
  }
}
function bytes(base64) {
  if (typeof base64 !== 'string' || base64.length > 4 * 1024 * 1024) throw new Error('MD-DISPLAY: payload bound');
  return Uint8Array.from(atob(base64), char => char.charCodeAt(0));
}
export async function supportedCodecs() {
  const codecs = ['png'];
  if (globalThis.VideoDecoder && (await VideoDecoder.isConfigSupported({ codec: VP9 })).supported) codecs.unshift(VP9);
  return codecs;
}
export class BrowserDisplayPresenter {
  constructor(canvas, binding, onTiming = () => {}) {
    this.canvas = canvas; this.binding = binding; this.sequence = 0; this.documentId = null;
    this.busy = false; this.closed = false;
    this.onTiming = onTiming;
  }
  async present(frame, shouldDraw = () => true) {
    if (this.closed) return false;
    if (this.busy) throw new Error('MD-DISPLAY: await presentation before granting next credit');
    if (frame.subscription_id !== this.binding.subscription_id || frame.generation !== this.binding.generation || frame.tab_id !== this.binding.tab_id || frame.sequence <= this.sequence) return false;
    if (!Number.isSafeInteger(frame.sequence) || ![1, 2].includes(frame.device_scale_factor) || frame.width !== 1280 * frame.device_scale_factor || frame.height !== 800 * frame.device_scale_factor || frame.css_width !== 1280 || frame.css_height !== 800 || typeof frame.document_id !== 'string' || !frame.document_id || frame.document_id.length > 256) throw new Error('MD-DISPLAY: invalid geometry/binding');
    if (frame.kind === 'tiles' && (frame.base_sequence !== this.sequence || frame.document_id !== this.documentId)) throw new Error('MD-DISPLAY: repair base lost; subscribe afresh');
    this.busy = true;
    const at = performance.timeOrigin + performance.now();
    const patch = frame.kind === 'tiles';
    if (patch && (this.canvas.width !== frame.width || this.canvas.height !== frame.height)) { this.busy = false; throw new Error('MD-DISPLAY: repair canvas changed'); }
    if(!patch && (!this.back || this.back.width !== frame.width || this.back.height !== frame.height))
      this.back=new OffscreenCanvas(frame.width,frame.height);
    const back=patch?null:this.back,context=back?.getContext('2d');
    const bitmaps = [];
    try {
      if (frame.kind === 'tiles') {
        if (!Array.isArray(frame.tiles) || frame.tiles.length > 260) throw new Error('MD-DISPLAY: tile count');
        for (const tile of frame.tiles) {
          if (![tile.x, tile.y, tile.width, tile.height].every(Number.isSafeInteger) || tile.x < 0 || tile.y < 0 || tile.width < 1 || tile.width > 128 || tile.height < 1 || tile.height > 128 || tile.x + tile.width > frame.width || tile.y + tile.height > frame.height) throw new Error('MD-DISPLAY: tile geometry');
          const bitmap = await createImageBitmap(new Blob([bytes(tile.data_base64)], { type: 'image/png' })); bitmaps.push(bitmap);
          if (bitmap.width !== tile.width || bitmap.height !== tile.height) throw new Error('MD-DISPLAY: tile dimensions');
        }
      } else if (frame.kind === 'png') {
        const bitmap = await createImageBitmap(new Blob([bytes(frame.data_base64)], { type: 'image/png' })); bitmaps.push(bitmap);
        if (bitmap.width !== frame.width || bitmap.height !== frame.height) throw new Error('MD-DISPLAY: image dimensions');
        context.drawImage(bitmap, 0, 0);
      } else if (frame.kind === 'video' && frame.codec === VP9 && typeof frame.key === 'boolean') {
        if (!this.decoder || frame.key) {
          this.decoder?.close();
          this.decoder = new VideoDecoder({ output: value => (this.decoded ? this.decoded.resolve(value) : value.close()), error: error => this.decoded?.reject(error) });
          this.decoder.configure({ codec:VP9, codedWidth:frame.width, codedHeight:frame.height, optimizeForLatency:true });
          this.videoSequence = null;
        } else if (this.videoSequence !== frame.sequence-1 || this.documentId !== frame.document_id) throw Error('MD-DISPLAY: video base lost; subscribe afresh');
        let output, timer;
        try {
          output = await new Promise((resolve,reject) => {
            timer=setTimeout(()=>reject(Error('MD-DISPLAY: decode timeout')),5000);
            this.decoded={resolve,reject};
            this.decoder.decode(new EncodedVideoChunk({type:frame.key?'key':'delta',timestamp:frame.sequence*33333,data:bytes(frame.data_base64)}));
          });
          if (!((output.displayWidth === frame.width && output.displayHeight === frame.height) || (output.displayWidth === 1280 && output.displayHeight === 800))) throw Error('MD-DISPLAY: decoded geometry');
          context.drawImage(output,0,0,frame.width,frame.height); this.videoSequence=frame.sequence;
        } finally { clearTimeout(timer); this.decoded=null; output?.close(); }
      } else throw new Error('MD-DISPLAY: unsupported frame');
      if (this.closed) return false;
      this.onTiming('client_decode', at);
      if (!shouldDraw()) {
        this.sequence=frame.sequence; this.documentId=frame.document_id; this.didDraw=false;
        this.onTiming('client_drop_stale', performance.timeOrigin+performance.now());
        return true;
      }
      this.didDraw=true;
      const presented = performance.timeOrigin + performance.now();
      if (patch) {
        // All patches are decoded and validated before this synchronous commit.
        const front = this.canvas.getContext('2d');
        for (let i = 0; i < bitmaps.length; i++) front.drawImage(bitmaps[i], frame.tiles[i].x, frame.tiles[i].y);
      } else {
        if(this.canvas.width !== frame.width)this.canvas.width=frame.width;
        if(this.canvas.height !== frame.height)this.canvas.height=frame.height;
        this.canvas.getContext('2d').drawImage(back, 0, 0);
      }
      this.sequence = frame.sequence; this.documentId = frame.document_id;
      this.onTiming('client_present', presented);
      return true;
    } finally { for (const bitmap of bitmaps) bitmap.close(); this.busy = false; }
  }
  input(input) {
    if (this.closed || !this.documentId) throw new Error('MD-DISPLAY: no displayed document');
    return { op: 'display_input', tab_id: this.binding.tab_id, generation: this.binding.generation, document_id: this.documentId, input };
  }
  close() { if(this.back){this.back.width=1;this.back.height=1;this.back=null;} this.decoder?.close(); this.decoder=null; this.closed = true; this.documentId = null; this.canvas.width = 1; this.canvas.height = 1; }
}
// One outstanding credit; events and responses can arrive in either order.
export async function attachBrowserDisplay(canvas, transport, tab, options = {}) {
  const request = async command => {
    const reply = await transport.request({ KernelBrowser: { command } });
    return reply.KernelBrowser.result;
  };
  const binding = { ...await request({ op: 'display_subscribe', ...tab, codecs: await supportedCodecs(), bitrate: options.bitrate ?? 2_000_000, device_scale_factor: options.deviceScaleFactor ?? 2 }), ...tab };
  const onTiming = options.onTiming ?? (() => {});
  const presenter = new BrowserDisplayPresenter(canvas, binding, onTiming);
  let pending = null, stopped = false, creditOutstanding = false;
  let running = false, failure = null, presentation = Promise.resolve(), active = new Set();
  const frames = [], arrivals = [];
  let latest = null,nextSequence=null,heldBytes=0;
  const held=new Map();
  let queuedBytes = 0;
  const accept = frame => {
    const size = JSON.stringify(frame).length;
    if (frames.length >= 8 || queuedBytes + size > 1024 * 1024) throw Error('MD-DISPLAY: credited receive window exceeded');
    latest=frame;
    const presented = presentation.then(async () => {
      if (!await presenter.present(frame,()=>frame.kind !== 'video' || latest.kind !== 'video' || latest.document_id !== frame.document_id || latest.sequence <= frame.sequence)) throw Error('MD-DISPLAY: rejected window frame');
      if(presenter.didDraw !== false) options.onPresented?.(frame);
    });
    presentation=presented;
    presented.catch(error => { failure=error; running=false; });
    const item={frame,presented,size};
    if (arrivals.length) arrivals.shift()(item);
    else { frames.push(item); queuedBytes += size; }
  };
  const receive = () => frames.length ? Promise.resolve((queuedBytes -= frames[0].size, frames.shift())) : new Promise(resolve => arrivals.push(resolve));
  const off = transport.onEvent(event => {
    if (event.event !== 'kernel_browser_frame' || event.subscription_id !== binding.subscription_id) return;
    if (running || active.size) {
      try {
        const frame=event.frame,size=JSON.stringify(frame).length;
        if(!Number.isSafeInteger(frame.sequence)||frame.sequence<nextSequence||held.has(frame.sequence)||held.size+frames.length>=8||heldBytes+queuedBytes+size>1024*1024) throw Error('MD-DISPLAY: reordered receive window exceeded');
        held.set(frame.sequence,frame);heldBytes+=size;
        // Relay control/large-event lanes and concurrent decryption may finish
        // in different orders. Decode every dependency in source sequence.
        while(held.has(nextSequence)) {
          const ordered=held.get(nextSequence);held.delete(nextSequence++);
          heldBytes-=JSON.stringify(ordered).length;accept(ordered);
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
    const desired=options.creditWindow ?? 4;
    if(!Number.isSafeInteger(desired)||desired<1||desired>8) {running=false;throw Error('MD-DISPLAY: invalid credit window');}
    const batch=Math.min(192000,Math.max(24000,(binding.bitrate??options.bitrate??2000000)/8*.5*.75-4096));
    const count=Math.min(desired,Math.max(1,Math.floor(1024*1024/(batch*4/3+4096))));
    for(let i=0;i<count;i++) issue();
  };
  const stop = async () => { running=false;idle.wake(); await Promise.allSettled([...active]); if(failure) throw failure; };
  return { binding, presenter, next, start, stop,
    get running() { return running; },
    input: input => {idle.wake();return request(presenter.input(input));},
    takeover: () => request({ op: 'display_takeover', ...tab }),
    release: () => request({ op: 'display_release', ...tab }),
    actors: () => request({ op: 'display_actors' }),
    async close() { await stop().catch(() => {}); stopped = true; pending?.reject(new Error('MD-DISPLAY: closed')); pending = null; off(); presenter.close(); await transport.unsubscribeDisplay?.(binding); await request({ op: 'unsubscribe', subscription_id: binding.subscription_id, generation: binding.generation }); },
  };
}
