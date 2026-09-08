import { AppError } from './errors.js';
import { object, token } from './protocol.js';
import { AppPeer } from './peer.js';
import { validateOccurrence, validateOccurrences } from './occurrences.js';

export { AppError } from './errors.js';
export { occurrenceId } from './occurrences.js';

const lifecycleNames = ['health_check', 'startup', 'suspend', 'resume', 'shutdown', 'prepare_update', 'configuration_change'];

function name(value, label = 'name') {
  if (!token(value)) throw new AppError('INVALID_ARGUMENT', `Invalid ${label}`);
  return value;
}

function record(value, label) {
  if (!object(value)) throw new AppError('INVALID_ARGUMENT', `Expected ${label}`);
  return value;
}

function relativePath(value) {
  if (typeof value !== 'string' || value.length === 0 || value.length > 4096
    || value.includes('\\') || value.startsWith('/') || /^[a-z]:/iu.test(value)
    || /[\x00-\x1f\x7f]/u.test(value) || value.split('/').some((part) => part === '' || part === '.' || part === '..')) {
    throw new AppError('INVALID_ARGUMENT', 'Expected a private relative file path');
  }
  return value;
}

function bytes(value) {
  if (typeof value !== 'string' && !(value instanceof Uint8Array)) {
    throw new AppError('INVALID_ARGUMENT', 'Expected text or bytes');
  }
  const data = typeof value === 'string' ? Buffer.from(value) : Buffer.from(value.buffer, value.byteOffset, value.byteLength);
  if (data.length > 512 * 1024) throw new AppError('INVALID_ARGUMENT', 'Inline App data exceeds 512 KiB');
  return data.toString('base64');
}

function registry(declared, label) {
  if (!Array.isArray(declared) || declared.length > 1024) throw new TypeError(`Invalid declared ${label}`);
  const allowed = new Set(declared.map((item) => name(item, label)));
  if (allowed.size !== declared.length) throw new TypeError(`Duplicate declared ${label}`);
  const handlers = new Map();
  let sealed = false;
  return {
    register(key, handler) {
      if (sealed) throw new AppError('ALREADY_READY', 'App registration has finished');
      if (!allowed.has(key)) throw new AppError('UNDECLARED_HANDLER', `Undeclared ${label}: ${key}`);
      if (handlers.has(key)) throw new AppError('DUPLICATE_HANDLER', `Already registered ${label}: ${key}`);
      if (typeof handler !== 'function') throw new TypeError('Expected an App handler');
      handlers.set(key, handler);
    },
    get(key) {
      const handler = handlers.get(key);
      if (!handler) throw new AppError('METHOD_NOT_FOUND', `Unregistered ${label}`);
      return handler;
    },
    names: () => [...handlers.keys()].sort(),
    complete: () => handlers.size === allowed.size,
    seal: () => { sealed = true; },
  };
}

/** Called by the trusted worker bootstrap after containment, never by a launcher. */
export function createAppSdk({ transport, generation, paths, declarations = {}, limits }) {
  for (const path of ['package', 'data', 'temporary']) {
    if (typeof paths?.[path] !== 'string' || paths[path].length === 0) throw new TypeError(`Missing ${path} root`);
  }
  record(declarations, 'trusted declarations');
  if (Object.keys(declarations).some(key => !['tools', 'incomingEvents'].includes(key))) {
    throw new TypeError('Invalid trusted declarations');
  }
  const tools = registry(declarations.tools ?? [], 'tool');
  // The trusted bootstrap selects incoming/both from signed declarations.
  // Outgoing occurrences use kernel-owned automations and need no local handler.
  const events = registry(declarations.incomingEvents ?? [], 'incoming event');
  const lifecycle = registry(lifecycleNames, 'lifecycle event');
  let lifecycleBusy = false;
  let ready = false;
  const peer = new AppPeer({
    transport, generation, limits,
    async handleRequest(method, params, context) {
      record(params, 'App invocation');
      switch (method) {
        case 'tools.invoke':
          name(params.name, 'tool name');
          if (!Object.hasOwn(params, 'input')) throw new AppError('INVALID_ARGUMENT', 'Missing tool input');
          return tools.get(params.name)(params.input, context);
        case 'events.deliver':
          name(params.name, 'event name');
          name(params.occurrence_id, 'occurrence identity');
          if (!Object.hasOwn(params, 'payload')) throw new AppError('INVALID_ARGUMENT', 'Missing event payload');
          return events.get(params.name)(Object.freeze({ occurrenceId: params.occurrence_id, payload: params.payload }), context);
        case 'lifecycle.dispatch':
          if (!lifecycleNames.includes(params.event)) throw new AppError('INVALID_ARGUMENT', 'Unknown lifecycle event');
          if (lifecycleBusy) throw new AppError('BUSY', 'An App lifecycle handler is still running', { retryable: true });
          lifecycleBusy = true;
          try {
            if (!lifecycle.names().includes(params.event)) return null;
            return await lifecycle.get(params.event)(params.data ?? null, context);
          } finally { lifecycleBusy = false; }
        default:
          throw new AppError('METHOD_NOT_FOUND', 'Unknown App worker operation');
      }
    },
  });
  const call = (method, params, options) => peer.request(method, params, options);
  const sdk = {
    paths: Object.freeze({ package: paths.package, data: paths.data, temporary: paths.temporary }),
    tools: Object.freeze({ register: tools.register }),
    events: Object.freeze({
      register: events.register,
      emit(occurrence, options) {
        return call('events.emit', validateOccurrence(occurrence), options);
      },
      status: (receiptId, options) => call('events.status', { receiptId: name(receiptId, 'receipt identity') }, options),
      retry: (receiptId, options) => call('events.retry', { receiptId: name(receiptId, 'receipt identity') }, options),
    }),
    lifecycle: Object.freeze({ on: lifecycle.register }),
    state: Object.freeze({
      get: (key, options) => call('state.get', { key: name(key, 'state key') }, options),
      transaction(transaction, options) {
        record(transaction, 'state transaction');
        if (transaction.occurrences !== undefined) {
          validateOccurrences(transaction.occurrences);
        }
        return call('state.transaction', transaction, options);
      },
    }),
    files: Object.freeze({
      atomicReplace: (path, contents, options) => call('files.atomic_replace', { path: relativePath(path), contentsBase64: bytes(contents) }, options),
      snapshot: (request, options) => call('files.snapshot', record(request, 'snapshot request'), options),
      import: (grantId, destination, options) => call('files.import', { grantId: name(grantId, 'file grant'), destination: relativePath(destination) }, options),
      export: (path, options) => call('files.export', { path: relativePath(path) }, options),
    }),
    http: Object.freeze({
      request(request, options) {
        record(request, 'HTTP request');
        let url;
        try { url = new URL(request.url); } catch { throw new AppError('INVALID_ARGUMENT', 'Invalid HTTP URL'); }
        if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
          throw new AppError('INVALID_ARGUMENT', 'Expected an HTTP URL without credentials');
        }
        const { body, ...parameters } = request;
        return call('http.request', { ...parameters, url: url.href, ...(body === undefined ? {} : { bodyBase64: bytes(body) }) }, options);
      },
    }),
    log: Object.freeze({
      write(level, message, fields = {}, options) {
        if (!['debug', 'info', 'warn', 'error'].includes(level) || typeof message !== 'string' || message.length > 4096) {
          throw new AppError('INVALID_ARGUMENT', 'Invalid App log record');
        }
        return call('log.write', { level, message, fields: record(fields, 'log fields') }, options);
      },
    }),
    host: Object.freeze({
      notify: (request, options) => call('host.notify', record(request, 'notification'), options),
      openLink: (url, options) => call('host.open_link', { url }, options),
      writeClipboard: (text, options) => call('host.clipboard_write', { text }, options),
      pickFile: (request, options) => call('host.pick_file', record(request, 'file picker request'), options),
    }),
    validation: Object.freeze({
      request: (request, options) => call('validation.request', record(request, 'human validation request'), options),
      status: (operationId, options) => call('validation.status', { operationId: name(operationId, 'operation identity') }, options),
    }),
    outputs: Object.freeze({
      request: (request, options) => call('outputs.request', record(request, 'information-set request'), options),
      cancel: (requestId, options) => call('outputs.cancel', { requestId: name(requestId, 'output request identity') }, options),
    }),
    async ready(options) {
      if (ready) throw new AppError('ALREADY_READY', 'App readiness already reported');
      if (!tools.complete() || !events.complete()) throw new AppError('MISSING_HANDLER', 'Declared App handlers are missing');
      // Mark before yielding; duplicate concurrent readiness cannot pass.
      ready = true;
      tools.seal();
      events.seal();
      lifecycle.seal();
      return call('worker.ready', { tools: tools.names(), events: events.names(), lifecycle: lifecycle.names() }, options);
    },
    close: () => peer.close(),
  };
  return Object.freeze(sdk);
}
