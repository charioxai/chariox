import { AppError } from './errors.js';
import { validateJson } from './json.js';
import { object, token } from './protocol.js';

const PAYLOAD_BYTES = 64 * 1024;
const INVOCATION_BYTES = 256 * 1024;
const BATCH_BYTES = 512 * 1024;

function invalid() { throw new AppError('INVALID_ARGUMENT', 'Invalid App occurrence'); }
function limit() { throw new AppError('LIMIT_EXCEEDED', 'App occurrence exceeds its limit'); }
function fields(value, required, optional = []) {
  if (!object(value) || required.some(key => !Object.hasOwn(value, key))
    || Object.keys(value).some(key => !required.includes(key) && !optional.includes(key))) invalid();
}
function text(value, bytes) {
  if (typeof value !== 'string' || !value.isWellFormed() || !/[^\p{White_Space}]/u.test(value)) invalid();
  if (Buffer.byteLength(value) > bytes) limit();
}
function encodedBytes(value, maximum) {
  try { validateJson(value); } catch { invalid(); }
  const size = Buffer.byteLength(JSON.stringify(value));
  if (size > maximum) limit();
  return size;
}

// Bound the walk before JSON encoding. The kernel repeats these bounds against
// the signed schema; SDK validation is developer feedback, never authority.
function payloadBytes(value) {
  let nodes = 16_384;
  let estimated = PAYLOAD_BYTES;
  const consume = bytes => { estimated -= bytes; if (estimated < 0) limit(); };
  const visit = (item, depth) => {
    if (--nodes < 0) limit();
    consume(1);
    if (typeof item === 'string') consume(Buffer.byteLength(item));
    else if (item !== null && typeof item === 'object') {
      if (depth >= 32) limit();
      if (Array.isArray(item)) {
        for (const entry of item) visit(entry, depth + 1);
      } else {
        for (const key of Object.keys(item)) {
          consume(Buffer.byteLength(key));
          visit(item[key], depth + 1);
        }
      }
    }
  };
  visit(value, 0);
  return encodedBytes(value, PAYLOAD_BYTES);
}

function invocationBytes(invocation) {
  fields(invocation, ['prompt', 'artifacts']);
  text(invocation.prompt, 64 * 1024);
  if (!Array.isArray(invocation.artifacts)) invalid();
  if (invocation.artifacts.length > 32) limit();
  for (const artifact of invocation.artifacts) {
    fields(artifact, ['name', 'mediaType', 'reference'], ['sizeBytes', 'digest']);
    for (const field of ['name', 'mediaType', 'reference']) {
      if (typeof artifact[field] !== 'string' || Buffer.byteLength(artifact[field]) > 2048) invalid();
      text(artifact[field], 2048);
    }
    if (Object.hasOwn(artifact, 'sizeBytes')
      && (!Number.isSafeInteger(artifact.sizeBytes) || artifact.sizeBytes < 0)) invalid();
    if (Object.hasOwn(artifact, 'digest')
      && (typeof artifact.digest !== 'string' || !/^sha256:[a-f0-9]{64}$/u.test(artifact.digest))) invalid();
  }
  // References remain untrusted metadata. This SDK performs no reads or fetches
  // and cannot promote a reference into a file, credential or attachment grant.
  return encodedBytes(invocation, INVOCATION_BYTES);
}

function measure(occurrence) {
  fields(occurrence, ['automationId', 'occurrenceId', 'eventVersion', 'occurredAtMs', 'payload', 'invocation'], ['scheduleRevision']);
  if (!token(occurrence.automationId) || !token(occurrence.occurrenceId)
    || !Number.isSafeInteger(occurrence.eventVersion) || occurrence.eventVersion < 1 || occurrence.eventVersion > 0xffffffff
    || !Number.isSafeInteger(occurrence.occurredAtMs) || occurrence.occurredAtMs < 0
    || (Object.hasOwn(occurrence, 'scheduleRevision') && !token(occurrence.scheduleRevision))) invalid();
  return payloadBytes(occurrence.payload) + invocationBytes(occurrence.invocation);
}

export function validateOccurrence(occurrence) {
  measure(occurrence);
  return occurrence;
}

export function validateOccurrences(occurrences) {
  if (!Array.isArray(occurrences)) invalid();
  if (occurrences.length > 16) limit();
  let bytes = 0;
  const identities = new Set();
  for (const occurrence of occurrences) {
    bytes += measure(occurrence);
    if (bytes > BATCH_BYTES) limit();
    const identity = JSON.stringify([occurrence.automationId, occurrence.occurrenceId,
      occurrence.scheduleRevision ?? null]);
    if (identities.has(identity)) invalid();
    identities.add(identity);
  }
  return occurrences;
}
