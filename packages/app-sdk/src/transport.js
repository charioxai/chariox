import { Socket } from 'node:net';
import { AppError } from './errors.js';
import { encodeFrame, FrameDecoder, MAX_FRAME_BYTES } from './protocol.js';

// Owns only an already-inherited channel. It never opens a socket address.
export function inheritedTransport(fd = 3, options = {}) {
  if (!Number.isSafeInteger(fd) || fd < 3) throw new RangeError('Expected an inherited App IPC descriptor');
  return streamTransport(new Socket({ fd, readable: true, writable: true }), options);
}

export function streamTransport(stream, { maxFrameBytes = MAX_FRAME_BYTES, maxQueuedBytes = 2 * MAX_FRAME_BYTES, frameTimeoutMs = 30_000 } = {}) {
  if (!Number.isSafeInteger(maxQueuedBytes) || maxQueuedBytes < maxFrameBytes + 4
    || maxQueuedBytes > 16 * MAX_FRAME_BYTES) throw new RangeError('Invalid App IPC write queue limit');
  if (!Number.isSafeInteger(frameTimeoutMs) || frameTimeoutMs < 1 || frameTimeoutMs > 30_000) {
    throw new RangeError('Invalid App IPC frame deadline');
  }
  let consumer;
  let disconnected;
  let closed = false;
  let queued = 0;
  let subscribed = false;
  let frameTimer;
  const writeTimers = new Set();
  const decoder = new FrameDecoder((message) => {
    clearTimeout(frameTimer);
    frameTimer = undefined;
    consumer(message);
  }, maxFrameBytes);
  const close = (reason = new AppError('DISCONNECTED', 'App IPC disconnected')) => {
    if (closed) return;
    closed = true;
    clearTimeout(frameTimer);
    for (const timer of writeTimers) clearTimeout(timer);
    writeTimers.clear();
    decoder.close();
    stream.off('data', data);
    stream.off('end', end);
    stream.off('close', onClose);
    stream.destroy();
    disconnected?.(reason);
  };
  const data = (chunk) => {
    try {
      decoder.push(chunk);
      if (!closed && decoder.hasPartialFrame && !frameTimer) {
        frameTimer = setTimeout(() => close(new AppError('PROTOCOL_ERROR', 'App IPC frame deadline exceeded')), frameTimeoutMs);
      }
    } catch (error) { close(error); }
  };
  const end = () => {
    try { decoder.end(); close(); } catch (error) { close(error); }
  };
  const onClose = () => close();
  // Attach before resuming reads, so an early stream error has a consumer.
  stream.on('error', close);
  stream.pause();
  return {
    subscribe(onMessage, onCloseCallback) {
      if (subscribed || closed) throw new AppError('DISCONNECTED', 'App IPC cannot be subscribed again');
      subscribed = true;
      consumer = onMessage;
      disconnected = onCloseCallback;
      stream.on('data', data);
      stream.on('end', end);
      stream.on('close', onClose);
      stream.resume();
      return () => close();
    },
    send(message) {
      if (closed) throw new AppError('DISCONNECTED', 'App IPC disconnected');
      const frame = encodeFrame(message, maxFrameBytes);
      if (queued + frame.length > maxQueuedBytes || writeTimers.size >= 128) {
        const error = new AppError('BACKPRESSURE', 'App IPC write queue is full');
        close(error);
        throw error;
      }
      queued += frame.length;
      const timer = setTimeout(() => close(new AppError('PROTOCOL_ERROR', 'App IPC write deadline exceeded')), frameTimeoutMs);
      writeTimers.add(timer);
      try {
        stream.write(frame, (error) => {
          clearTimeout(timer);
          writeTimers.delete(timer);
          queued -= frame.length;
          if (error) close(error);
        });
      } catch (error) {
        clearTimeout(timer);
        writeTimers.delete(timer);
        queued -= frame.length;
        close(error);
        throw error;
      }
    },
    close,
  };
}
