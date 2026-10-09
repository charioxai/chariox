// MP-08/MP-10: hosted-equivalent click/type echo on real public pages.
// Same echo criteria as the hosted campaign: Wikipedia theme background
// luminance for click, the portal search glyph template for typing. Echo is
// judged in the presenter's draw callback (drawn_ms) and the next rAF.
import { distribution } from './drill-metrics.mjs';

const ARTICLE = 'https://en.wikipedia.org/wiki/Web_browser', PORTAL = 'https://www.wikipedia.org/';

async function command(page, op, extra = {}) {
  return page.evaluate(({ op, extra }) => mdTransport.request({ KernelBrowser: { command: { op, tab_id: mdStream.binding.tab_id, generation: mdStream.binding.generation, ...extra } } }), { op, extra });
}
async function controls(page, ids) {
  const reply = await command(page, 'snapshot');
  const snapshot = reply.KernelBrowser.result.snapshot, dpr = await page.evaluate(() => mdStream.binding.device_scale_factor);
  return Object.fromEntries((snapshot.dom_nodes ?? []).filter(n => ids.includes(n.attributes?.id) && n.bounds?.width > 0)
    .map(n => [n.attributes.id, Object.fromEntries(Object.entries(n.bounds).map(([k, v]) => [k, v / dpr]))]));
}
const centre = b => ({ x: Math.round(b.x + b.width / 2), y: Math.round(b.y + b.height / 2) });

// One probe: fire the input (not awaiting its reply, as a viewer does) and
// wait until a presented frame satisfies `test` over the ROI.
async function probe(page, input, roi, test, args) {
  return page.evaluate(async ({ input, roi, test, args }) => {
    const stamp = () => performance.timeOrigin + performance.now(), dpr = mdStream.binding.device_scale_factor;
    const check = new Function('pixels', 'args', test);
    const read = () => MDDisplay.canvas.getContext('2d', { willReadFrequently: true }).getImageData(Math.floor(roi.x * dpr), Math.floor(roi.y * dpr), Math.max(1, Math.floor(roi.width * dpr)), Math.max(1, Math.floor(roi.height * dpr))).data;
    const frames = [];
    const seen = new Promise((resolve, reject) => {
      const timer = setTimeout(() => { window.mdOnPresented = null; reject(Error('MP-10: site echo timeout')); }, 10000);
      window.mdOnPresented = sample => {
        const arrival = mdFrames.findLast(f => f.sequence === sample.sequence);
      frames.push({ sequence: sample.sequence, kind: sample.kind, drawn_ms: sample.drawn_ms, arrived_ms: arrival?.arrived_ms, bytes: arrival?.bytes });
        if (!check(read(), args)) return;
        window.mdOnPresented = null; clearTimeout(timer);
        requestAnimationFrame(() => resolve({ ...sample, raf_ms: stamp() }));
      };
    });
    const started = stamp();
    mdStream.input(input).catch(() => {});
    const echo = await seen;
    return { started_ms: started, echo_sequence: echo.sequence, echo_kind: echo.kind, drawn_ms: echo.drawn_ms, raf_ms: echo.raf_ms,
      latency_ms: echo.drawn_ms - started, raf_latency_ms: echo.raf_ms - started, frames };
  }, { input, roi, test, args });
}
const luminance = `let n=0;for(let i=0;i<pixels.length;i+=4)n+=(pixels[i]+pixels[i+1]+pixels[i+2])/3;const m=n/(pixels.length/4);return args.dark?m<100:m>180;`;
const glyph = `let good=0,bad=0;for(const i of args.mask){good+=(pixels[i]-args.want[i])**2;bad+=(pixels[i]-args.other[i])**2;}return good<bad*.35;`;

export async function measureSiteLatency({ page, pause, pair, samples = 40 }) {
  const result = { items: ['MP-08', 'MP-10'], qualification: 'local relay + netem; hosted-equivalent echo criteria; drawn_ms is the presenter draw, raf_ms the next animation frame', click_samples: [], type_samples: [] };
  await command(page, 'navigate', { url: ARTICLE }); await pause(10000);
  result.article_view = await pair('site-article');
  // A fundraising banner can push the Appearance menu below the viewport;
  // close it as a reader would (its top-right close button at scroll zero).
  const ids = ['skin-client-pref-skin-theme-value-day', 'skin-client-pref-skin-theme-value-night'];
  let theme = await controls(page, ids);
  if (theme[ids[1]]?.y > 700) {
    result.banner = { before: theme[ids[1]] ?? null };
    await page.evaluate(() => mdStream.input({ kind: 'click', x: 1214, y: 97 })); await pause(3000);
    theme = await controls(page, ids); result.banner.after = theme[ids[1]] ?? null; result.banner_view = await pair('site-article-closed');
  }
  const day = theme[ids[0]], night = theme[ids[1]];
  if (!day || !night || night.y + night.height > 790) throw Error('MP-10: Wikipedia theme controls absent from the viewport');
  const roi = { x: 2, y: 100, width: 10, height: 10 };
  await probe(page, { kind: 'click', ...centre(day) }, roi, luminance, { dark: false }).catch(() => null);
  for (let n = 0; n < samples; n++) {
    const dark = n % 2 === 0;
    result.click_samples.push({ index: n, dark, ...await probe(page, { kind: 'click', ...centre(dark ? night : day) }, roi, luminance, { dark }) });
  }
  await command(page, 'navigate', { url: PORTAL }); await pause(8000);
  result.portal_view = await pair('site-portal');
  const search = (await controls(page, ['searchInput'])).searchInput;
  if (!search) throw Error('MP-10: portal search control absent');
  await page.evaluate(input => mdStream.input(input), { kind: 'click', ...centre(search) }); await pause(1500);
  const box = { x: search.x + 4, y: search.y + 5, width: 40, height: Math.max(8, search.height - 10) };
  const read = () => page.evaluate(roi => { const dpr = mdStream.binding.device_scale_factor; return Array.from(MDDisplay.canvas.getContext('2d').getImageData(Math.floor(roi.x * dpr), Math.floor(roi.y * dpr), Math.floor(roi.width * dpr), Math.floor(roi.height * dpr)).data); }, box);
  const empty = await read();
  await page.evaluate(() => mdStream.input({ kind: 'text', text: 'a' })); await pause(2000);
  const typed = await read();
  await page.evaluate(() => mdStream.input({ kind: 'key', key: 'Backspace' })); await pause(2000);
  const mask = []; for (let i = 0; i < typed.length; i++) if (i % 4 !== 3 && Math.abs(typed[i] - empty[i]) > 60) mask.push(i);
  if (mask.length < 20) throw Error('MP-10: portal glyph templates not distinct');
  result.glyph_components = mask.length;
  for (let n = 0; n < samples; n++) {
    const a = n % 2 === 0;
    result.type_samples.push({ index: n, kind: a ? 'type' : 'backspace',
      ...await probe(page, a ? { kind: 'text', text: 'a' } : { kind: 'key', key: 'Backspace' }, box, glyph, { mask, want: a ? typed : empty, other: a ? empty : typed }) });
  }
  for (const key of ['click', 'type']) {
    result[key + '_drawn'] = distribution(result[key + '_samples'].map(s => s.latency_ms));
    result[key + '_raf'] = distribution(result[key + '_samples'].map(s => s.raf_latency_ms));
  }
  return result;
}
