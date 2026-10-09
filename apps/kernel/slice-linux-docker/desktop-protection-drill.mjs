// MP-08 / MP-10 / MP-11: kernel Chromium on the owned Linux desktop, observed
// through the production Computer screenshot and desktop stream paths.
// Usage: node desktop-protection-drill.mjs <evidence-dir> [url...]
// Without URLs it serves the magenta/cyan regression fixture (never acceptance).
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './docker/kernel-browser-host.mjs';
import { DesktopSource } from './docker/kernel-desktop-source.mjs';
import { decodePng, encodePng } from './docker/kernel-browser-pixels.mjs';
import { serveFixture } from './docker/browser-protection-fixture.mjs';

const [evidence, ...urls] = process.argv.slice(2);
assert.ok(evidence, 'evidence directory required');
await mkdir(evidence, { recursive: true });
const root = await mkdtemp(path.join(os.tmpdir(), 'desktop-protection-'));
const fixture = urls.length ? null : await serveFixture();
const policy = { values: [], targets: [], unknown: false };
const host = new KernelBrowserHost(root);
const report = [];
let source;
// Exact-color census over RGBA; content rows exclude the 87px browser chrome.
function census(width, height, rgba) {
  let magenta = 0, cyan = 0, visible = 0, total = 0;
  for (let y = 100; y < height; y++) for (let x = 0; x < width; x++) {
    const i = (y * width + x) * 4, r = rgba[i], g = rgba[i + 1], b = rgba[i + 2];
    total++; if (r || g || b) visible++;
    if (r === 255 && !g && b === 255) magenta++;
    if (!r && g === 255 && b === 255) cyan++;
  }
  return { magenta, cyan, visible_fraction: Number((visible / total).toFixed(4)) };
}
const slug = url => url.replace(/^https?:\/\//, '').replace(/[^a-z0-9]+/gi, '-').slice(0, 60);
try {
  const call = async params => {
    const reply = await host.handle({ id: Math.random(), method: 'host.computer', params });
    assert.equal(reply.ok, true, JSON.stringify(reply.error));
    return reply.result;
  };
  const state = await call({ op: 'start' });
  for (const url of urls.length ? urls : [fixture.url + '?novault']) {
    const entry = { url };
    try {
      const opened = await host.request({ op: 'open', url });
      entry.tab = opened.tab_id ?? null;
      await delay(urls.length ? 6000 : 1500);
      const started = performance.now();
      const shot = await call({ op: 'screenshot', surface_id: state.surface_id, generation: state.generation });
      entry.screenshot_ms = Math.round(performance.now() - started);
      const png = decodePng(shot.data_base64, 1);
      entry.screenshot = { protected: shot.protected, ...census(png.width, png.height, png.pixels) };
      await writeFile(path.join(evidence, `${slug(url)}-screenshot.png`), Buffer.from(shot.data_base64, 'base64'));
      // Desktop stream: frames are released only under a verified measurement.
      const binding = host.chromium.desktop.binding();
      source = await new DesktopSource(binding, policy).start();
      const begin = performance.now();
      let frame = null;
      const off = source.subscribe(sample => { frame = sample; });
      for (let i = 0; i < 100 && !(frame && census(frame.width, frame.height, bgra(frame.raw.pixels)).visible_fraction > 0.2); i++) {
        await source.wake(); await delay(50);
      }
      off();
      frame ??= source.sample();
      const rgba = bgra(frame.raw.pixels);
      entry.stream = { first_revealed_ms: Math.round(performance.now() - begin), ...census(frame.width, frame.height, rgba) };
      await writeFile(path.join(evidence, `${slug(url)}-stream.png`), Buffer.from(encodePng(frame.width, frame.height, rgba), 'base64'));
      // Real wheel input through the Computer path while the stream runs:
      // every published frame must stay protected; report rate and withholds.
      const frames = [];
      const stop = source.subscribe(sample => frames.push(sample));
      const scrollStarted = performance.now();
      for (let i = 0; i < 40; i++) {
        await call({ op: 'input', surface_id: state.surface_id, generation: state.generation, input: { kind: 'scroll', x: 640, y: 450, steps: i < 20 ? 1 : -1 } });
        await delay(50);
      }
      const seconds = (performance.now() - scrollStarted) / 1000;
      stop();
      const counts = frames.map(sample => census(sample.width, sample.height, bgra(sample.raw.pixels)));
      entry.scroll = { seconds: Number(seconds.toFixed(2)), frames: frames.length, fps: Number((frames.length / seconds).toFixed(1)),
        max_magenta: Math.max(0, ...counts.map(count => count.magenta)), withheld_frames: counts.filter(count => count.visible_fraction < 0.5).length };
      await source.close(); source = null;
    } catch (error) { entry.error = String(error?.message ?? error).slice(0, 300); }
    report.push(entry);
    console.log(JSON.stringify(entry));
  }
} finally {
  await source?.close();
  await host.stop();
  await fixture?.close();
  await rm(root, { recursive: true, force: true });
  await writeFile(path.join(evidence, 'report.json'), JSON.stringify(report, null, 1));
}
function bgra(pixels) {
  const rgba = Buffer.from(pixels);
  for (let i = 0; i < rgba.length; i += 4) { const blue = rgba[i]; rgba[i] = rgba[i + 2]; rgba[i + 2] = blue; rgba[i + 3] = 255; }
  return rgba;
}
