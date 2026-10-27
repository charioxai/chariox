export class AppError extends Error {
  constructor(code, message, options = {}) {
    super(message, options);
    this.name = 'AppError';
    this.code = code;
    this.retryable = options.retryable === true;
  }
}

export function protocolError(message) {
  return new AppError('PROTOCOL_ERROR', message);
}

export function wireError(error) {
  // App exceptions can contain paths, credentials or other private details.
  // Explicit AppErrors are suitable for callers; unclassified errors are not.
  // App code can alter an AppError's fields: reading them must not throw, and
  // a code the wire refuses fails the call as HANDLER_FAILED (see AppPeer).
  try {
    if (error instanceof AppError && typeof error.code === 'string' && typeof error.message === 'string') {
      return { code: error.code, message: boundedMessage(error.message), retryable: error.retryable === true };
    }
  } catch { /* an App-made error object */ }
  return { code: 'HANDLER_FAILED', message: 'App handler failed', retryable: false };
}

function boundedMessage(message) {
  let length = 0;
  let result = '';
  for (const character of message.toWellFormed()) {
    length += Buffer.byteLength(character);
    if (length > 4096) break;
    result += character;
  }
  return result;
}
