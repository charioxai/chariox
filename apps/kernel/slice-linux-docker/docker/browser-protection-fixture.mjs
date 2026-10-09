// MP-08/MP-11 regression fixture (opt-in browser tests only): protected
// content is pure magenta, ordinary content (consent dialog, a cross-site
// "reCAPTCHA" frame, article text) pure cyan. Never an acceptance substitute.
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { connectCdpPipe } from './kernel-browser-cdp-pipe.mjs';
import { BrowserCdpClient } from './browser-controller-cdp.mjs';

export async function launchChromium({ executable, dpr = 1, headless = true, env = process.env, root, args = [] }) {
  const profile = await mkdtemp(path.join(root, 'chromium-'));
  const child = spawn(executable, [`--user-data-dir=${profile}`, '--remote-debugging-pipe', '--no-first-run', '--no-default-browser-check',
    '--disable-background-networking', `--force-device-scale-factor=${dpr}`, '--window-size=1000,760', ...(headless ? ['--headless=new'] : []),
    // Test harness only: Chromium refuses its sandbox for uid 0 (root builders).
    ...(process.getuid?.() === 0 ? ['--no-sandbox'] : []), ...args, 'about:blank'],
  { env, stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe'] });
  child.on('error', () => {});
  const connection = connectCdpPipe(child.stdio[3], child.stdio[4], 10000);
  const browser = new BrowserCdpClient({ connectionFactory: () => connection });
  await connection.send('Browser.getVersion');
  return { connection, browser, child, async close() {
    await connection.send('Browser.close').catch(() => {});
    for (let i = 0; i < 50 && child.exitCode === null && child.signalCode === null; i++) await delay(20);
    if (child.exitCode === null && child.signalCode === null && Number.isSafeInteger(child.pid) && child.pid > 1) child.kill('SIGKILL');
    await rm(profile, { recursive: true, force: true });
  } };
}

// Exact-color census of an RGBA buffer, optionally after painting regions black.
export function census({ width, pixels }, regions = []) {
  const masked = Buffer.from(pixels);
  for (const [x, y, w, h] of regions) for (let py = Math.max(0, y); py < Math.min(pixels.length / 4 / width, y + h); py++) for (let px = Math.max(0, x); px < Math.min(width, x + w); px++) masked.fill(0, (py * width + px) * 4, (py * width + px) * 4 + 3);
  let magenta = 0, cyan = 0;
  for (let i = 0; i < masked.length; i += 4) {
    if (masked[i] === 255 && masked[i + 1] === 0 && masked[i + 2] === 255) magenta++;
    if (masked[i] === 0 && masked[i + 1] === 255 && masked[i + 2] === 255) cyan++;
  }
  return { magenta, cyan };
}
