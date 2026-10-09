// MD-DISPLAY-04: MP-10 protocol 443 presentation only. Cloud supplies its existing
// admitted/encrypted kernel request and event adapter; never a Cloud media proxy.
export const minimumProtocolVersion = 443;
const VP9 = 'vp09.00.10.08';
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
  async present(frame) {
    if (this.closed) return false;
    if (this.busy) throw new Error('MD-DISPLAY: await presentation before granting next credit');
    if (frame.subscription_id !== this.binding.subscription_id || frame.generation !== this.binding.generation || frame.tab_id !== this.binding.tab_id || frame.sequence <= this.sequence) return false;
    if (!Number.isSafeInteger(frame.sequence) || ![1, 2].includes(frame.device_scale_factor) || frame.width !== 1280 * frame.device_scale_factor || frame.height !== 800 * frame.device_scale_factor || frame.css_width !== 1280 || frame.css_height !== 800 || typeof frame.document_id !== 'string' || !frame.document_id || frame.document_id.length > 256) throw new Error('MD-DISPLAY: invalid geometry/binding');
    if (frame.kind === 'tiles' && (frame.base_sequence !== this.sequence || frame.document_id !== this.documentId)) throw new Error('MD-DISPLAY: repair base lost; subscribe afresh');
    this.busy = true;
    const at = performance.timeOrigin + performance.now();
    const patch = frame.kind === 'tiles';
    if (patch && (this.canvas.width !== frame.width || this.canvas.height !== frame.height)) { this.busy = false; throw new Error('MD-DISPLAY: repair canvas changed'); }
    const back = patch ? null : new OffscreenCanvas(frame.width, frame.height), context = back?.getContext('2d');
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
      } else if (frame.kind === 'video' && frame.codec === VP9 && frame.key === true) {
        let output, decodeError;
        const decoder = new VideoDecoder({ output: value => { output?.close(); output = value; }, error: error => { decodeError = error; } });
        try {
          decoder.configure({ codec: VP9, codedWidth: frame.width, codedHeight: frame.height });
          decoder.decode(new EncodedVideoChunk({ type: 'key', timestamp: frame.sequence * 200000, data: bytes(frame.data_base64) }));
          await decoder.flush();
          if (decodeError) throw decodeError;
          if (!output || output.displayWidth !== frame.width || output.displayHeight !== frame.height) throw new Error('MD-DISPLAY: decoded geometry');
          context.drawImage(output, 0, 0);
        } finally { output?.close(); if (decoder.state !== 'closed') decoder.close(); }
      } else throw new Error('MD-DISPLAY: unsupported frame');
      if (this.closed) return false;
      this.onTiming('client_decode', at);
      const presented = performance.timeOrigin + performance.now();
      if (patch) {
        // All patches are decoded and validated before this synchronous commit.
        const front = this.canvas.getContext('2d');
        for (let i = 0; i < bitmaps.length; i++) front.drawImage(bitmaps[i], frame.tiles[i].x, frame.tiles[i].y);
      } else {
        this.canvas.width = frame.width; this.canvas.height = frame.height;
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
  close() { this.closed = true; this.documentId = null; this.canvas.width = 1; this.canvas.height = 1; }
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
  const off = transport.onEvent(event => {
    if (event.event !== 'kernel_browser_frame' || event.subscription_id !== binding.subscription_id) return;
    pending?.resolve(event.frame); pending = null;
  });
  try { await transport.subscribeDisplay?.(binding); }
  catch (error) {
    off(); presenter.close();
    await request({ op: 'unsubscribe', subscription_id: binding.subscription_id, generation: binding.generation }).catch(() => {});
    throw error;
  }
  const next = async () => {
    if (stopped || creditOutstanding) throw new Error('MD-DISPLAY: stream stopped or credit outstanding');
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
  return { binding, presenter, next,
    input: input => request(presenter.input(input)),
    takeover: () => request({ op: 'display_takeover', ...tab }),
    release: () => request({ op: 'display_release', ...tab }),
    actors: () => request({ op: 'display_actors' }),
    async close() { stopped = true; pending?.reject(new Error('MD-DISPLAY: closed')); pending = null; off(); presenter.close(); await transport.unsubscribeDisplay?.(binding); await request({ op: 'unsubscribe', subscription_id: binding.subscription_id, generation: binding.generation }); },
  };
}
