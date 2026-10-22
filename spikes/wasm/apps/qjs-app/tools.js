// The same tool bodies for the componentize-js App and the plain Node App.
// `io` supplies counter/fetch over each runtime's own host calls.
export const TOOLS = ['echo', 'counter', 'fetch', 'transform', 'compute', 'spin', 'hog'];

export function transform(n = 1000, rounds = 1) {
  const records = [];
  for (let i = 0; i < n; i++) records.push({ id: i, name: `item-${i}`, score: (i * 7919) % 100, tags: ['a', 'b', 'c'] });
  const text = JSON.stringify(records);
  let sum = 0;
  let outBytes = 0;
  for (let r = 0; r < rounds; r++) {
    const picked = JSON.parse(text).filter(x => x.score > 50).map(x => { sum += x.score; return { id: x.id, name: x.name.toUpperCase() }; });
    outBytes = JSON.stringify(picked).length;
  }
  return { sum, inBytes: text.length, outBytes };
}

// Interpreter-bound work: no JSON builtins, string keys and a Map.
export function compute(n = 1000000) {
  let h = 0;
  const counts = new Map();
  for (let i = 0; i < n; i++) {
    const key = 'k' + (i % 1000);
    counts.set(key, (counts.get(key) || 0) + 1);
    h = (Math.imul(h, 31) + i) | 0;
  }
  return { h, keys: counts.size };
}

export function spin() {
  let x = 0;
  for (;;) x = (x + 1) | 0;
}

// `bytes` grows byte buffers; `objects` grows small heap objects (~100 B each).
export function hog(mb = 64, kind = 'bytes') {
  const chunks = [];
  if (kind === 'objects') {
    for (let i = 0; i < mb * 10000; i++) chunks.push({ i, s: 'x'.repeat(64) + i });
    return { allocatedMb: mb, objects: chunks.length };
  }
  for (let i = 0; i < mb; i++) chunks.push(new Uint8Array(1 << 20).fill(i % 251));
  return { allocatedMb: chunks.length };
}
