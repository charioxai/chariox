// The App page's typed backend call, carried over private MessagePorts: page -> trusted host -> embedding client.
// The client owns the connection and its authority; this file only frames, bounds and correlates calls.
export const CALL = 'chariox.app-view.call';
export const RESULT = 'chariox.app-view.result';
export const MAX_METHOD_LENGTH = 256;
export const MAX_INPUT_BYTES = 1024 * 1024;
export const MAX_PENDING_CALLS = 16;

const plain = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const callNumber = value => Number.isSafeInteger(value) && value >= 0;
const sameKeys = (value, keys) => Object.keys(value).sort().join() === keys;

export function readCall(data) {
  if (!plain(data) || !sameKeys(data, 'id,input,method,type') || data.type !== CALL || !callNumber(data.id)
      || typeof data.method !== 'string' || !data.method || data.method.length > MAX_METHOD_LENGTH) return null;
  try {
    const encoded = JSON.stringify(data.input);
    if (typeof encoded !== 'string' || encoded.length > MAX_INPUT_BYTES) return null;
  } catch { return null; }
  return { id: data.id, method: data.method, input: data.input };
}

export function readAnswer(data) {
  if (!plain(data) || data.type !== RESULT || !callNumber(data.id) || typeof data.ok !== 'boolean') return null;
  if (data.ok) return sameKeys(data, 'id,ok,result,type') ? { id: data.id, ok: true, result: data.result } : null;
  const { error } = data;
  if (!sameKeys(data, 'error,id,ok,type') || !plain(error) || !sameKeys(error, 'code,message')
      || typeof error.code !== 'string' || typeof error.message !== 'string') return null;
  return { id: data.id, ok: false, error: { code: error.code, message: error.message } };
}

export const answerOk = (id, result) => ({ type: RESULT, id, ok: true, result });
export const answerError = (id, code, message) => ({ type: RESULT, id, ok: false, error: { code, message } });

// A kernel refusal carries a code and a sentence written for the person; anything else is not shown.
function failure(error) {
  const code = typeof error?.code === 'string' && /^[A-Za-z0-9_.-]{1,64}$/.test(error.code) ? error.code : 'CALL_FAILED';
  const message = code !== 'CALL_FAILED' && typeof error?.message === 'string' && error.message ? error.message.slice(0, 512) : 'The call did not complete';
  return { code, message };
}

/** Client side: answer each call on `port` with `call(method, input)`. Returns a function that stops serving. */
export function serveCalls(port, call) {
  let pending = 0;
  port.onmessage = async event => {
    const request = readCall(event.data);
    if (!request) return;
    if (pending >= MAX_PENDING_CALLS) { port.postMessage(answerError(request.id, 'TOO_MANY_CALLS', 'The App has too many calls in flight')); return; }
    pending++;
    let answer;
    try { answer = answerOk(request.id, await call(request.method, request.input)); }
    catch (error) { const { code, message } = failure(error); answer = answerError(request.id, code, message); }
    finally { pending--; }
    try { port.postMessage(answer); } catch { port.postMessage(answerError(request.id, 'CALL_FAILED', 'The call did not complete')); }
  };
  return () => { port.onmessage = null; };
}

/** Host side: forward the page's calls to the client under host-chosen numbers, so a page cannot collide or forge an answer. */
export function relayCalls(app, client) {
  const waiting = new Map();
  let next = 0, stopped = false;
  app.onmessage = event => {
    const request = readCall(event.data);
    if (!request) {
      const id = event.data?.id;
      if (plain(event.data) && event.data.type === CALL && callNumber(id)) app.postMessage(answerError(id, 'INVALID_CALL', 'The call is not a method name and a JSON input'));
      return;
    }
    if (waiting.size >= MAX_PENDING_CALLS) { app.postMessage(answerError(request.id, 'TOO_MANY_CALLS', 'The App has too many calls in flight')); return; }
    const hostId = next++;
    waiting.set(hostId, request.id);
    client.postMessage({ type: CALL, id: hostId, method: request.method, input: request.input });
  };
  client.onmessage = event => {
    const answer = readAnswer(event.data);
    const pageId = answer && waiting.get(answer.id);
    if (pageId === undefined) return;
    waiting.delete(answer.id);
    app.postMessage(answer.ok ? answerOk(pageId, answer.result) : answerError(pageId, answer.error.code, answer.error.message));
  };
  return () => {
    if (stopped) return;
    stopped = true;
    app.onmessage = null; client.onmessage = null;
    for (const pageId of waiting.values()) app.postMessage(answerError(pageId, 'VIEW_CLOSED', 'The App view closed'));
    waiting.clear();
  };
}
