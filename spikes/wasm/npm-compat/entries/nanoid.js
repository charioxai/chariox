import { nanoid, customAlphabet } from 'nanoid';

// Ids are random, so the result reports shape, not value.
export function run() {
  const ids = Array.from({ length: 100 }, () => nanoid());
  const hex = customAlphabet('0123456789abcdef', 12)();
  return JSON.stringify({
    len21: ids.every((id) => id.length === 21),
    alphabet: ids.every((id) => /^[A-Za-z0-9_-]{21}$/.test(id)),
    unique: new Set(ids).size === 100,
    hex12: /^[0-9a-f]{12}$/.test(hex),
  });
}
