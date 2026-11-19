// The built window in headless Chrome against the App's real code and the loopback GitHub of support.mjs. The page
// calls its backend through `chariox.call`, which here checks the input against the signed schema the way the kernel
// does before the App runs.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createApp } from '../src/app.mjs';
import { fakeGitHub, memoryState } from './support.mjs';

const root = new URL('..', import.meta.url).pathname;
export const CHROME = process.env.CHARIOX_TEST_CHROME ?? ['/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser'].find(existsSync);
const declared = Object.fromEntries(JSON.parse(readFileSync(join(root, 'bundle/schemas/tools.json'), 'utf8')).tools.map(tool => [tool.name, tool.inputSchema]));

/** The signed input schema, as far as the window can break it: unknown keys, missing keys, values outside an enum. */
function check(name, input) {
  const schema = declared[name];
  if (!schema) throw { code: 'UNKNOWN_TOOL', message: name };
  for (const key of Object.keys(input ?? {})) if (!(key in schema.properties)) throw { code: 'INVALID_ARGUMENT', message: `${name}: unexpected ${key}` };
  for (const key of schema.required ?? []) if (!(key in (input ?? {}))) throw { code: 'INVALID_ARGUMENT', message: `${name}: missing ${key}` };
  for (const [key, value] of Object.entries(input ?? {})) {
    const rule = schema.properties[key];
    if (rule.enum && !rule.enum.includes(value)) throw { code: 'INVALID_ARGUMENT', message: `${name}: ${key} not in enum` };
    if (rule.pattern && typeof value === 'string' && !new RegExp(rule.pattern).test(value)) throw { code: 'INVALID_ARGUMENT', message: `${name}: ${key} does not match` };
  }
}

export function buildWindow() {
  const out = mkdtempSync(join(tmpdir(), 'github-window-'));
  execFileSync(process.execPath, [join(root, 'scripts/build-package.mjs'), join(out, 'app')], { stdio: 'pipe' });
  return {
    script: readFileSync(`${out}/app-build/app.js`, 'utf8'), style: readFileSync(`${out}/app-build/app.css`, 'utf8'),
    remove: () => { rmSync(out, { recursive: true, force: true }); rmSync(`${out}-build`, { recursive: true, force: true }); },
  };
}

export async function launch() {
  const { chromium } = createRequire(join(root, 'package.json'))('playwright-core');
  return chromium.launch({ executablePath: CHROME, args: ['--no-sandbox'] });
}

/** `override(url, init)` answers a request first, as in `fakeGitHub`; `calls` lists every request the App made. */
export async function open(browser, built, { dpr = 1, width = 1180, height = 900, override, unbound, imports = {}, now = Date.parse('2026-11-15T12:00:00Z') } = {}) {
  const double = fakeGitHub({ override, unbound });
  const app = createApp({ state: memoryState(), events: {}, ...imports, now: () => now });
  const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: dpr, colorScheme: 'light' });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  const tools = [];
  await page.exposeFunction('__call', async (name, input) => {
    tools.push({ name, input });
    try {
      check(name, input);
      if (imports.routing) imports.routing.callers = [{ kind: 'human', id: 'owner' }];
      return JSON.stringify({ ok: true, value: await app.tools[name](input, { fetch: double.fetch }) });
    } catch (error) {
      const cause = error?.payload ?? error;
      return JSON.stringify({ ok: false, error: { code: cause.code, message: cause.message, resetAt: cause.resetAt } });
    }
  });
  const shim = 'window.chariox = { call: async (name, input) => { const r = JSON.parse(await window.__call(name, input)); if (r.ok) return r.value; throw Object.assign(new Error(r.error.message), r.error) } }';
  await page.setContent(`<!doctype html><html><head><meta charset="utf-8"><script>${shim}</script><style>${built.style}</style></head><body><div id="github"></div><script type="module">${built.script.replace(/<\/script/gi, '<\\/script')}</script></body></html>`);
  return { page, errors, app, double, context, tools, changes: () => double.calls.filter(call => call.method !== 'GET') };
}
