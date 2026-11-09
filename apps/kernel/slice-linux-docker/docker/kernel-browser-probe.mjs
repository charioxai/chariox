#!/usr/bin/env node
// MP-08/MP-10/MP-11: manual probe of the kernel browser against stock Chrome, run before every staging.
//
//   node kernel-browser-probe.mjs --executable /path/to/chrome [--docker-dir DIR] [--arms stock:headful,kernel:headful,kernel:headless]
//        [--checks signals,google-email-step,browserscan,...] [--out DIR]
//
// It starts the real kernel browser host (KernelBrowserHost from --docker-dir, default: this directory) and a stock Chrome of the
// same build, runs every check in each, and prints a pass/fail table. A kernel arm passes when its outcome equals the stock
// outcome (so a site that blocks this network blocks both and still passes); "rejected" by Google is always a failure.
// Google: one email attempt per arm with a throwaway address that does not exist. Never a real account, never a password.
// Google may answer repeated attempts from one network with a CAPTCHA: that is the network, not the browser (outcome "captcha", warn).
// macOS: the kernel browser is always headless there, run with --arms stock:headful,kernel:headless; Linux needs Xvfb for stock:headful.
// Exit code 1 when any check fails. Screenshots go to --out.
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { withBrowser, collectSignals } from './kernel-browser-probe-arms.mjs';
import { unexpectedDifferences, unexpectedHeaderDifferences } from './kernel-browser-signals.mjs';
import { checks } from './kernel-browser-probe-checks.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const option = (name, fallback) => { const i = process.argv.indexOf(`--${name}`); return i > 0 ? process.argv[i + 1] : fallback; };
const executable = option('executable', process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE);
if (!executable) { console.error('usage: kernel-browser-probe.mjs --executable <Chrome binary> (the same build the kernel runs)'); process.exit(2); }
const dockerDir = option('docker-dir', here);
const out = option('out', path.join(process.cwd(), 'kernel-browser-probe-out'));
const defaultArms = process.platform === 'darwin' ? 'stock:headful,kernel:headless' : 'stock:headful,kernel:headful,kernel:headless';
const arms = option('arms', defaultArms).split(',').map(a => { const [arm, mode] = a.split(':'); return { arm, mode, name: a }; });
const wanted = option('checks', ['signals', ...checks.map(c => c.id)].join(',')).split(',');
await mkdir(out, { recursive: true });
const scratchRoot = option('scratch', out);

const results = {}; // check -> arm name -> {outcome, detail}
const record = (check, arm, value) => { (results[check] ??= {})[arm.name] = value; };

if (wanted.includes('signals')) {
  const per = {};
  for (const arm of arms) per[arm.name] = await collectSignals({ arm: arm.arm, mode: arm.mode, dockerDir, executable, scratchRoot });
  const stock = per['stock:headful'];
  for (const arm of arms) {
    if (arm.arm === 'stock') { record('signals', arm, { outcome: stock ? `${Object.keys(stock.signals).length} signals` : '-', detail: stock?.version ?? '' }); continue; }
    const diffs = stock ? unexpectedDifferences(stock.signals, per[arm.name].signals) : [];
    const headers = stock ? unexpectedHeaderDifferences(stock.seen, per[arm.name].seen, arm.mode === 'headless' ? ['sec-ch-ua-full-version-list', 'sec-ch-ua-arch', 'sec-ch-ua-bitness', 'sec-ch-ua-model', 'sec-ch-ua-platform-version'] : []) : [];
    const unexpected = [...diffs.filter(d => arm.mode === 'headless' || d.key !== 'uad.high'), ...headers.map(h => ({ key: `header ${h.key}` }))];
    record('signals', arm, { outcome: unexpected.length ? `DIFF ${unexpected.map(d => d.key).join(',')}` : 'equal to stock except listed', detail: per[arm.name].version, fail: unexpected.length > 0 });
  }
  await writeFile(path.join(out, 'signals.json'), JSON.stringify(per, null, 1));
}

for (const arm of arms) {
  const external = checks.filter(c => wanted.includes(c.id));
  if (!external.length) continue;
  await withBrowser({ arm: arm.arm, mode: arm.mode, dockerDir, executable, scratchRoot }, async page => {
    for (const check of external) {
      try { record(check.id, arm, await check.run(page)); await writeFile(path.join(out, `${check.id}-${arm.arm}-${arm.mode}.png`), await page.screenshot()); }
      catch (error) { record(check.id, arm, { outcome: `error: ${String(error.message).slice(0, 60)}`, detail: '', error: true }); }
    }
  });
}

let failed = 0;
const rows = [];
for (const [id, byArm] of Object.entries(results)) {
  const stock = byArm['stock:headful']?.outcome;
  for (const arm of arms.filter(a => a.arm === 'kernel')) {
    const r = byArm[arm.name]; if (!r) continue;
    let verdict = 'PASS';
    if (id === 'signals') verdict = r.fail ? 'FAIL' : 'PASS';
    else if (r.error) verdict = 'FAIL';
    else if (id === 'google-email-step' && r.outcome === 'rejected') verdict = 'FAIL';
    else if (stock !== undefined && r.outcome !== stock) verdict = id === 'google-email-step' || id === 'recaptcha-v3' ? 'WARN' : 'FAIL';
    if (verdict === 'FAIL') failed++;
    rows.push([id, arm.name, verdict, r.outcome, stock ?? '-']);
  }
}
const pad = (s, n) => String(s).padEnd(n);
console.log(`${pad('check', 20)} ${pad('arm', 16)} ${pad('verdict', 7)} ${pad('kernel outcome', 44)} stock outcome`);
for (const r of rows) console.log(`${pad(r[0], 20)} ${pad(r[1], 16)} ${pad(r[2], 7)} ${pad(r[3].slice(0, 43), 44)} ${r[4].slice(0, 43)}`);
await writeFile(path.join(out, 'table.json'), JSON.stringify({ rows, results }, null, 1));
console.log(failed ? `\n${failed} FAIL` : '\nall checks PASS (WARN: not caused by the browser, read the outcome)');
process.exit(failed ? 1 : 0);
