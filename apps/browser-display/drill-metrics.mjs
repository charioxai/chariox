// MD-DISPLAY-02: RGB fidelity and explicit latency distributions.
export function distribution(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const percentile = (p) => sorted[Math.min(sorted.length - 1, Math.ceil(p * sorted.length) - 1)] ?? null;
  const histogram = [0, 16, 33, 50, 100, 200, 500, 1000].map((min, i, edges) => ({
    min_ms: min, max_ms: edges[i + 1] ?? null,
    count: values.filter((x) => x >= min && (edges[i + 1] === undefined || x < edges[i + 1])).length,
  }));
  return { n: values.length, p50_ms: percentile(.5), p95_ms: percentile(.95), p99_ms: percentile(.99), max_ms: sorted.at(-1) ?? null, histogram, samples_ms: values };
}

export function compare(reference, actual, PNG) {
  const a = PNG.sync.read(reference), b = PNG.sync.read(actual);
  if (a.width !== b.width || a.height !== b.height) return { comparable: false, reference: [a.width, a.height], actual: [b.width, b.height] };
  let error = 0, different = 0;
  const diff = new PNG({ width: a.width, height: a.height });
  // PSNR over RGB. This intentionally includes text, borders, and opaque patches.
  for (let i = 0; i < a.data.length; i += 4) {
    let changed = false;
    for (let c = 0; c < 3; c++) {
      const d = Math.abs(a.data[i + c] - b.data[i + c]);
      error += d * d; changed ||= d > 3; diff.data[i + c] = Math.min(255, d * 4);
    }
    different += Number(changed); diff.data[i + 3] = 255;
  }
  const mse = error / (a.width * a.height * 3);
  return { comparable: true, width: a.width, height: a.height, mse_rgb: mse, psnr_db: mse === 0 ? null : 10 * Math.log10(255 * 255 / mse), lossless: mse === 0, changed_pixels_fraction: different / (a.width * a.height), diff: PNG.sync.write(diff) };
}
