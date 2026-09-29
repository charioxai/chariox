import { protocolError } from './errors.js';

export const MAX_JSON_DEPTH = 64;
const MAX_JSON_BYTES = 1024 * 1024;

/** JSON shared with Rust: finite numbers, safe integers and Unicode scalars. */
export function validateJson(value) {
  const parents = new Set();
  const path = [];
  let bytes = 0;
  // The error names where the value is, so an App developer can find it.
  const invalid = (problem) => jsonError(`${where(path)} ${problem}`);
  const add = (count) => {
    bytes += count;
    if (bytes > MAX_JSON_BYTES) throw jsonError('the message exceeds its 1 MiB size limit');
  };
  const string = (text, key = false) => {
    if (!text.isWellFormed()) {
      throw invalid(key ? 'has a key that is not well-formed Unicode' : 'is a string that is not well-formed Unicode');
    }
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
        if (!Number.isFinite(item)) throw invalid(`is ${item}, not a finite number`);
        if (Number.isInteger(item) && !Number.isSafeInteger(item)) {
          throw invalid('is an integer beyond the safe range; send it as a string');
        }
        add(String(item).length);
        return;
      case 'object':
        if (depth >= MAX_JSON_DEPTH) throw invalid(`exceeds the nesting limit of ${MAX_JSON_DEPTH}`);
        if (parents.has(item)) throw invalid('refers back to an object that contains it (a cycle)');
        if (!Array.isArray(item) && ![Object.prototype, null].includes(Object.getPrototypeOf(item))) {
          throw invalid('is not a plain object or array');
        }
        parents.add(item);
        add(2);
        if (Array.isArray(item)) {
          for (let index = 0; index < item.length; index += 1) {
            if (index) add(1);
            path.push(index);
            visit(item[index], depth + 1);
            path.pop();
          }
        } else {
          const keys = Object.keys(item);
          for (let index = 0; index < keys.length; index += 1) {
            if (index) add(1);
            string(keys[index], true);
            add(1);
            path.push(keys[index]);
            visit(item[keys[index]], depth + 1);
            path.pop();
          }
        }
        parents.delete(item);
        return;
      case 'undefined': throw invalid('is undefined');
      case 'bigint': throw invalid('is a BigInt');
      case 'function': throw invalid('is a function');
      case 'symbol': throw invalid('is a symbol');
      default: throw invalid('is not a JSON value');
    }
  };
  visit(value, 0);
  return value;
}

/** A protocol error whose `detail` says which value is not JSON, and why. */
export function jsonError(detail) {
  const error = protocolError(`Invalid App IPC JSON: ${detail}`);
  error.detail = detail;
  return error;
}

const IDENTIFIER = /^[A-Za-z_$][\w$]*$/u;

// A bounded, readable path such as `result.items[2].name`. Keys are App data:
// each is shortened by code point, so the text stays well-formed Unicode.
function where(path) {
  let text = '';
  for (const key of path) {
    let part;
    if (typeof key === 'number') part = `[${key}]`;
    else {
      const shown = [...key].slice(0, 64).join('');
      part = IDENTIFIER.test(shown) ? `.${shown}` : `[${JSON.stringify(shown)}]`;
    }
    if (text.length + part.length > 256) return `${text.replace(/^\./u, '')}…`;
    text += part;
  }
  return text.replace(/^\./u, '') || 'the value';
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
