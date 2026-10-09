// MP-08/MP-11: run adversarial snapshots off-thread so a synchronous grid
// regression has a real deadline, rather than blocking node:test's timeout.
import test from 'node:test';
import assert from 'node:assert/strict';
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';
import { documentProtection } from './browser-protection-regions.mjs';
import { locateBrowserRegions } from './browser-observation-regions.mjs';
import { RENDER_ORDER_STYLES } from './browser-controller-snapshot.mjs';
import { DEFAULT_RENDER_STYLE } from './browser-protection-fixture.mjs';

function snapshot(kind) {
  const strings = [], id = value => { const at = strings.indexOf(value); return at < 0 ? strings.push(value) - 1 : at; };
  const n = kind === 'huge' ? 1 : 2000;
  const big = kind === 'huge' ? [-1e6, -1e6, 2e6, 2e6] : kind === 'cells' ? [0, 0, 800, 700] : [10, 10, 10, 20];
  const names = ['#document', 'DIV', ...Array(n).fill('#text'), 'P', '#text'];
  const parentIndex = [-1, 0, ...Array(n).fill(1), 0, n + 2];
  const ordinary = [900, 20, 100, 30], container = kind === 'huge' ? big : [0, 0, 800, 700];
  const bounds = [[0, 0, 1100, 750], container, ...Array(n).fill(big), ordinary, ordinary];
  const text = [-1, -1, ...Array(n).fill(id('x')), -1, id('ordinary')];
  const nodes = { nodeName: names.map(id), parentIndex, nodeValue: text, backendNodeId: names.map((_, i) => i + 1) };
  const layout = { nodeIndex: names.map((_, i) => i), bounds, text, styles: names.map((name, i) => name === '#document' ? [] : RENDER_ORDER_STYLES.map(p => id({ ...DEFAULT_RENDER_STYLE, ...(i >= 1 && i <= n + 1 ? { transform: 'matrix(100000,0,0,100000,0,0)' } : {}) }[p]))) };
  return { strings, documents: [{ nodes, layout, scrollOffsetX: 0, scrollOffsetY: 0,
    textBoxes: { layoutIndex: Array.from({ length: n }, (_, i) => i + 2), bounds: Array(n).fill(big) } }] };
}

async function collect(kind, collector) {
  const raw = snapshot(kind), viewport = [0, 0, 1100, 750];
  if (collector === 'documentProtection') return documentProtection(raw, 0, { values: ['unrelated-value'], viewport }).regions;
  const connection = { async send(method) {
    if (method === 'Target.getTargets') return { targetInfos: [] };
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'root', loaderId: 'document' } } };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 1 };
    if (method === 'Runtime.evaluate') return { result: { value: ['visible', 1, 750] } };
    if (method === 'Page.getLayoutMetrics') return { cssLayoutViewport: { clientWidth: 1100, clientHeight: 750, pageX: 0, pageY: 0 }, cssVisualViewport: { scale: 1 } };
    if (method === 'DOMSnapshot.captureSnapshot') return raw;
    throw new Error(method);
  } };
  const browser = { async resolvePageTarget() { return { connection, sessionId: 'session' }; } };
  return locateBrowserRegions([{ target_id: 'target', echo_only: true }], browser, ['unrelated-value'], { contentTarget: 'target' });
}

if (!isMainThread) {
  const start = performance.now();
  parentPort.postMessage({ regions: await collect(workerData.kind, workerData.collector), elapsed: performance.now() - start });
} else {
  for (const collector of ['documentProtection', 'locateBrowserRegions']) for (const kind of ['huge', 'cells', 'comparisons']) {
    test(`MP-08/MP-11 ${collector}: ${kind} grid stays bounded and fallback stays local`, { timeout: 5000 }, async () => {
      const worker = new Worker(new URL(import.meta.url), { workerData: { kind, collector }, resourceLimits: { maxOldGenerationSizeMb: 128 } });
      let deadline;
      try {
        const result = await new Promise((resolve, reject) => {
          deadline = setTimeout(() => reject(new Error('MP-11 collector exceeded 2 second deadline')), 2000);
          worker.once('message', resolve); worker.once('error', reject);
          worker.once('exit', code => { if (code) reject(new Error(`MP-11 worker exited ${code}`)); });
        });
        assert(result.elapsed < 1500, `MP-11 collector took ${result.elapsed} ms`);
        if (kind !== 'huge') {
          assert(result.regions.some(([x,y,w,h]) => x <= 10 && y <= 10 && x + w >= 800 && y + h >= 700), 'MP-11 exhausted proof covers its container');
          assert(!result.regions.some(([x,y,w,h]) => x <= 950 && y <= 25 && x + w > 950 && y + h > 25), 'MP-08 unrelated sibling stays visible');
        }
      } finally { clearTimeout(deadline); await worker.terminate(); }
    });
  }
}
