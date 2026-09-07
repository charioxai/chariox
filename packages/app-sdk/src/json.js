import { protocolError } from './errors.js';

export const MAX_JSON_DEPTH = 64;
const MAX_JSON_BYTES = 1024 * 1024;

/** JSON shared with Rust: finite numbers, safe integers and Unicode scalars. */
export function validateJson(value) {
  const parents = new Set();
  let bytes = 0;
  const add = (count) => {
    bytes += count;
    if (bytes > MAX_JSON_BYTES) throw protocolError('App IPC JSON exceeds size limit');
  };
  const string = (text) => {
    if (!text.isWellFormed()) throw protocolError('App IPC strings require valid Unicode scalars');
    add(2 + Buffer.byteLength(text));
    for (const character of text) {
      const code = character.codePointAt(0);
      if (code === 34 || code === 92) add(1);
      else if (code < 32) add([8, 9, 10, 12, 13].includes(code) ? 1 : 5);
    }
  };
  const visit = (item, depth) => {
    if (item === null) { add(4); return; }
    switch (typeof item) {
      case 'string': string(item); return;
      case 'boolean': add(item ? 4 : 5); return;
      case 'number':
        if (!Number.isFinite(item) || (Number.isInteger(item) && !Number.isSafeInteger(item))) {
          throw protocolError('App IPC numbers must be finite with safe integers');
        }
        add(String(item).length);
        return;
      case 'object':
        if (depth >= MAX_JSON_DEPTH) throw protocolError('App IPC JSON nesting exceeds limit');
        if (parents.has(item)) throw protocolError('App IPC JSON cannot contain cycles');
        if (!Array.isArray(item) && ![Object.prototype, null].includes(Object.getPrototypeOf(item))) {
          throw protocolError('App IPC requires plain JSON objects');
        }
        parents.add(item);
        add(2);
        if (Array.isArray(item)) {
          for (let index = 0; index < item.length; index += 1) {
            if (index) add(1);
            visit(item[index], depth + 1);
          }
        } else {
          const keys = Object.keys(item);
          for (let index = 0; index < keys.length; index += 1) {
            if (index) add(1);
            string(keys[index]);
            add(1);
            visit(item[keys[index]], depth + 1);
          }
        }
        parents.delete(item);
        return;
      default:
        throw protocolError('App IPC requires JSON values');
    }
  };
  visit(value, 0);
  return value;
}

/** Inspect nesting and duplicate object keys before JSON.parse drops duplicates. */
function inspectText(text) {
  const containers = [];
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index];
    if (character === '{' || character === '[') {
      containers.push(character === '{' ? new Set() : null);
      if (containers.length > MAX_JSON_DEPTH) throw protocolError('App IPC JSON nesting exceeds limit');
    } else if (character === '}' || character === ']') {
      containers.pop(); // JSON.parse below validates matching punctuation.
    } else if (character === '"') {
      const start = index;
      for (index += 1; index < text.length; index += 1) {
        if (text[index] === '\\') index += 1;
        else if (text[index] === '"') break;
      }
      let next = index + 1;
      while (next < text.length && /[\x20\t\n\r]/u.test(text[next])) next += 1;
      if (text[next] === ':' && containers.at(-1) instanceof Set) {
        const key = JSON.parse(text.slice(start, index + 1));
        if (containers.at(-1).has(key)) throw protocolError('Duplicate App IPC JSON key');
        containers.at(-1).add(key);
      }
    }
  }
}

export function parseJson(text) {
  try {
    inspectText(text);
    return validateJson(JSON.parse(text));
  } catch (error) {
    if (error?.code === 'PROTOCOL_ERROR') throw error;
    throw protocolError('Invalid App IPC JSON');
  }
}
