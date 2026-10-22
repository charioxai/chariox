const now = () => (typeof performance !== 'undefined' && performance.now ? performance.now() : NaN);

function timed(fn) {
  const d0 = Date.now();
  const p0 = now();
  const result = fn();
  return `date_ms=${Date.now() - d0} perf_ms=${(now() - p0).toFixed(1)} result=${result}`;
}

export function compute() {
  return timed(() => {
    const counts = new Map();
    let h = 0;
    for (let i = 0; i < 1000000; i++) {
      const key = 'k' + (i % 1000);
      counts.set(key, (counts.get(key) || 0) + 1);
      h = (Math.imul(h, 31) + i) | 0;
    }
    return `${counts.size}/${counts.get('k7')}/${h}`;
  });
}

export function json() {
  return timed(() => {
    const records = [];
    for (let i = 0; i < 10000; i++) {
      records.push({ id: i, name: `item-${i}`, score: (i * 7919) % 100, tags: ['a', 'b', 'c'] });
    }
    let text = JSON.stringify(records);
    let out = '';
    for (let r = 0; r < 5; r++) {
      const rows = JSON.parse(text).filter((x) => x.score > 50).map((x) => ({ id: x.id, name: x.name.toUpperCase() }));
      out = JSON.stringify(rows);
    }
    return `${text.length}/${out.length}`;
  });
}
