// One hop owns its kernel handle and in-flight broker calls. Redirects share
// the original outer deadline; cancellation never retries consumed body bytes.
import { CHUNK_BYTES, UPLOAD_LIMIT, unsupported } from './fetch-request.js';

export class Transfer {
  constructor(http, signal, deadline) {
    this.http = http;
    this.signal = signal;
    this.deadline = deadline;
    this.controller = new AbortController();
    this.closed = false;
    this.abort = () => this.stop(signal.reason ?? new DOMException('Aborted', 'AbortError'));
    signal.addEventListener('abort', this.abort, { once: true });
    this.timer = setTimeout(() => this.stop(new DOMException('HTTP lifetime exceeded', 'TimeoutError')), Math.max(0, deadline - performance.now()));
    this.timer.unref?.();
    if (signal.aborted) this.abort();
  }
  options() {
    if (this.failure) throw this.failure;
    if (this.closed) throw new DOMException('Aborted', 'AbortError');
    const timeoutMs = Math.min(30_000, Math.floor(this.deadline - performance.now()));
    if (timeoutMs < 1) throw new DOMException('HTTP lifetime exceeded', 'TimeoutError');
    return { timeoutMs, signal: this.controller.signal };
  }
  async open(value) {
    const opened = await this.http.open({ url: value.url.href, method: value.method,
      headers: [...value.headers], hasBody: value.request.body !== null }, this.options());
    this.id = opened.streamId;
    if (this.closed) {
      await this.cancelHandle();
      throw this.failure ?? new DOMException('Aborted', 'AbortError');
    }
  }
  async cancelHandle() {
    if (this.id && !this.cancelledHandle) {
      this.cancelledHandle = true;
      await Promise.resolve().then(() => this.http.cancel(this.id, { timeoutMs: 1000 })).catch(() => {});
    }
  }
  stop(error) {
    if (this.closed) return this.cleanup ?? Promise.resolve();
    this.closed = true;
    this.failure = error;
    clearTimeout(this.timer);
    this.signal.removeEventListener('abort', this.abort);
    this.controller.abort(error);
    if (error) { try { this.output?.error(error); } catch { /* already settled */ } }
    void this.uploadReader?.cancel(error).catch(() => {});
    this.cleanup = this.cancelHandle();
    return this.cleanup;
  }
  startUpload(body, expectedBytes) {
    if (!body) return;
    this.expectedBytes = expectedBytes;
    this.uploadReader = body.getReader();
    this.upload = this.sendBody().catch(error => {
      if (!this.closed && !this.networkDone) return this.stop(this.signal.aborted ? this.signal.reason : error);
    });
  }
  async endNetwork() {
    // A final response can finish while its request upload is still blocked.
    // Ending that upload is cleanup, not a failure of the received response.
    this.networkDone = true;
    this.controller.abort();
    void this.uploadReader?.cancel().catch(() => {});
    await this.cancelHandle();
  }
  async sendBody() {
    let total = 0;
    try {
      for (;;) {
        this.options();
        const { value, done } = await this.uploadReader.read();
        if (done) {
          if (this.expectedBytes !== undefined && total !== this.expectedBytes) throw unsupported('content-length does not match the request body');
          await this.http.write(this.id, new Uint8Array(), true, this.options());
          return;
        }
        if (!(value instanceof Uint8Array)) throw unsupported('upload stream must yield byte chunks');
        total += value.byteLength;
        if (total > UPLOAD_LIMIT) throw unsupported('request body exceeds 64 MiB');
        if (this.expectedBytes !== undefined && total > this.expectedBytes) throw unsupported('content-length does not match the request body');
        for (let offset = 0; offset < value.byteLength; offset += CHUNK_BYTES) {
          await this.http.write(this.id, value.subarray(offset, offset + CHUNK_BYTES), false, this.options());
        }
      }
    } finally { this.uploadReader.releaseLock(); }
  }
  async headers() {
    for (;;) {
      const head = await this.http.headers(this.id, this.options());
      if (!head.pending) return head;
      // A broken/fast pending implementation must still yield to abort/timers.
      await new Promise(resolve => setTimeout(resolve, 0));
    }
  }
  rawBody() {
    const owner = this;
    return new ReadableStream({
      async pull(controller) {
        try {
          for (;;) {
            const part = await owner.http.read(owner.id, owner.options());
            if (part.pending) { await new Promise(resolve => setTimeout(resolve, 0)); continue; }
            if (part.done) { controller.close(); await owner.endNetwork(); return; }
            if (typeof part.chunkBase64 !== 'string' || part.chunkBase64.length > Math.ceil(CHUNK_BYTES / 3) * 4) throw unsupported('invalid response chunk');
            const bytes = Buffer.from(part.chunkBase64, 'base64');
            if (bytes.length > CHUNK_BYTES || bytes.toString('base64') !== part.chunkBase64) throw unsupported('invalid response chunk encoding');
            controller.enqueue(new Uint8Array(bytes));
            return;
          }
        } catch (error) { controller.error(error); await owner.stop(error); }
      },
      cancel: reason => owner.stop(reason),
    }, { highWaterMark: 0 });
  }
}
