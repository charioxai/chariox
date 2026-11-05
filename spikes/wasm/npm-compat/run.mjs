// Builds each npm entry with QuickJS (async world) and StarlingMonkey (sync world),
// runs it in wasmtime and compares with Node. Then runs the compute/JSON benchmark.
// Usage: node run.mjs [compat|bench|all]   (scratch lives in /var/tmp/wasmspike/npm)
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const WORK = '/var/tmp/wasmspike/npm';
const WASMTIME = `${WORK}/wasmtime/wasmtime-v50.0.0-x86_64-linux/wasmtime`;
const PACKAGES = ['zod', 'date-fns', 'lodash-es', 'nanoid', 'marked'];
const SRC = `${WORK}/src`;
const OUT = `${WORK}/out`;
const env = { ...process.env, TZ: 'UTC' };

function sh(cmd, args, opts = {}) {
  const t0 = performance.now();
  const r = spawnSync(cmd, args, { encoding: 'utf8', env, timeout: 300_000, maxBuffer: 64 << 20, ...opts });
  return { ...r, secs: (performance.now() - t0) / 1000, text: `${r.stdout ?? ''}${r.stderr ?? ''}` };
}

function firstError(text) {
  const exception = /message: Some\("([^"]*)"\)/.exec(text)?.[1];
  if (exception) return `JS exception: ${exception}`;
  const cause = /Caused by:\s*\n\s*0: ([^\n]*)/.exec(text)?.[1];
  if (cause) return cause.slice(0, 200);
  const syntax = /SyntaxError[^\n]*\n[^\n]*/.exec(text)?.[0];
  if (syntax) return syntax.replace(/\n\S*/, ' ').slice(0, 200);
  const lines = text.split('\n').map((l) => l.trim()).filter(Boolean);
  return (lines.find((l) => /error|panic|caused by|failed|trap|unreachable/i.test(l) && !/^warning/i.test(l)) ?? lines[0] ?? '').slice(0, 200);
}

// wasmtime prints the returned string WAVE-encoded.
function decodeWave(s) {
  s = s.trim();
  if (!s.startsWith('"')) return null;
  return s.slice(1, -1).replace(/\\(u\{([0-9a-fA-F]+)\}|.)/g, (_, c, hex) =>
    hex ? String.fromCodePoint(parseInt(hex, 16)) : ({ n: '\n', t: '\t', r: '\r' })[c] ?? c);
}

// jco rewrites the WIT file it is given, so every build gets a fresh copy.
function freshWit(tag) {
  const dir = `${WORK}/wit/${tag}`;
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  copyFileSync(join(HERE, 'npm.wit'), `${dir}/npm.wit`);
  return `${dir}/npm.wit`;
}

function componentize(entry, backend, world, out, bundle) {
  const args = ['componentize', '-w', freshWit(out.split('/').pop()), '-n', world, '-o', out, entry];
  if (backend === 'quickjs') args.push('--backend', 'quickjs');
  if (bundle) args.push('--bundle');
  rmSync(out, { force: true });
  const r = sh('jco', args);
  const ok = r.status === 0;
  return { ok, secs: r.secs, size: ok ? statSync(out).size : null, error: ok ? '' : firstError(r.text) };
}

function invoke(wasm, call, timed = false) {
  const args = ['run', '-W', 'component-model-async=y', '-S', 'http', '--invoke', call, wasm];
  const r = timed ? sh('/usr/bin/time', ['-v', WASMTIME, ...args]) : sh(WASMTIME, args);
  const rss = /Maximum resident set size \(kbytes\): (\d+)/.exec(r.stderr ?? '')?.[1];
  const value = r.status === 0 ? decodeWave(r.stdout) : null;
  return { ok: value !== null, value, secs: r.secs, rssKb: rss ? Number(rss) : null, error: value === null ? firstError(r.text) : '' };
}

function nodeRun(file, fn) {
  const code = `const m = await import(${JSON.stringify(file)}); process.stdout.write(JSON.stringify(await m.${fn}()));`;
  const r = sh(process.execPath, ['--input-type=module', '-e', code], { cwd: WORK });
  return r.status === 0 ? JSON.parse(r.stdout) : `NODE ERROR: ${firstError(r.text)}`;
}

function prepareSources() {
  mkdirSync(SRC, { recursive: true });
  mkdirSync(OUT, { recursive: true });
  writeFileSync(`${SRC}/package.json`, '{ "type": "module" }\n');
  for (const f of [...PACKAGES, 'bench']) copyFileSync(join(HERE, 'entries', `${f}.js`), `${SRC}/${f}.js`);
  // The async world needs async exports; the wrapper keeps one entry per package.
  for (const p of PACKAGES) writeFileSync(`${SRC}/${p}.async.js`, `import { run as r } from './${p}.js';\nexport async function run() { return r(); }\n`);
  writeFileSync(`${SRC}/bench.async.js`, `import * as b from './bench.js';\nexport async function compute() { return b.compute(); }\nexport async function json() { return b.json(); }\n`);
}

// Self-bundled path: plain Rolldown, browser conditions, no Node-global adapters injected.
function rolldown(entry, out) {
  const r = sh('rolldown', [entry, '--format', 'esm', '--platform', 'browser', '-o', out], { cwd: WORK });
  if (r.status !== 0) throw new Error(`rolldown ${entry}: ${firstError(r.text)}`);
  return out;
}

function compat() {
  const rows = [];
  for (const p of PACKAGES) {
    const expected = nodeRun(`${SRC}/${p}.js`, 'run');
    const variants = [
      { tool: 'quickjs', bundler: 'jco --bundle', entry: `${SRC}/${p}.async.js`, world: 'npm-async', bundle: true },
      { tool: 'starlingmonkey', bundler: 'jco --bundle', entry: `${SRC}/${p}.js`, world: 'npm', bundle: true },
      { tool: 'quickjs', bundler: 'rolldown', entry: rolldown(`${SRC}/${p}.async.js`, `${OUT}/${p}.async.bundle.js`), world: 'npm-async', bundle: false },
      { tool: 'starlingmonkey', bundler: 'rolldown', entry: rolldown(`${SRC}/${p}.js`, `${OUT}/${p}.bundle.js`), world: 'npm', bundle: false },
    ];
    for (const v of variants) {
      const wasm = `${OUT}/${p}.${v.tool === 'quickjs' ? 'qjs' : 'sm'}.${v.bundle ? 'jcobundle' : 'rolldown'}.wasm`;
      const build = componentize(v.entry, v.tool, v.world, wasm, v.bundle);
      const run = build.ok ? invoke(wasm, 'run()') : { ok: false, error: 'not built' };
      const row = { pkg: p, tool: v.tool, bundler: v.bundler, build, run, matches: run.ok && run.value === expected, expected };
      rows.push(row);
      console.log(JSON.stringify({ pkg: p, tool: v.tool, bundler: v.bundler, built: build.ok, buildSecs: +build.secs.toFixed(2), size: build.size, buildError: build.error, ran: run.ok, runError: run.error, matches: row.matches, got: run.value?.slice(0, 160) }));
      if (run.ok && !row.matches) console.log(`  node: ${String(expected).slice(0, 300)}\n  wasm: ${run.value.slice(0, 300)}`);
    }
  }
  writeFileSync(`${WORK}/compat.json`, JSON.stringify(rows, null, 2));
}

function bench() {
  const targets = [
    { tool: 'quickjs', entry: `${SRC}/bench.async.js`, world: 'bench-async' },
    { tool: 'starlingmonkey', entry: `${SRC}/bench.js`, world: 'bench' },
  ];
  const results = [];
  for (const t of targets) {
    const wasm = `${OUT}/bench.${t.tool}.wasm`;
    const build = componentize(t.entry, t.tool, t.world, wasm, false);
    console.log(JSON.stringify({ bench: t.tool, built: build.ok, size: build.size, buildSecs: +build.secs.toFixed(2), error: build.error }));
    if (!build.ok) continue;
    for (const fn of ['compute', 'json']) {
      for (let i = 0; i < 3; i++) {
        const r = invoke(wasm, `${fn}()`, fn === 'compute');
        const row = { tool: t.tool, fn, i, out: r.value ?? r.error, processSecs: +r.secs.toFixed(2), rssKb: r.rssKb };
        results.push(row);
        console.log(JSON.stringify(row));
      }
    }
  }
  for (const fn of ['compute', 'json']) {
    for (let i = 0; i < 3; i++) {
      const r = sh('/usr/bin/time', ['-v', process.execPath, '--input-type=module', '-e',
        `const m = await import(${JSON.stringify(`${SRC}/bench.js`)}); console.log(m.${fn}());`]);
      const rss = /Maximum resident set size \(kbytes\): (\d+)/.exec(r.stderr)?.[1];
      const row = { tool: 'node', fn, i, out: r.stdout.trim(), processSecs: +r.secs.toFixed(2), rssKb: fn === 'compute' ? Number(rss) : null };
      results.push(row);
      console.log(JSON.stringify(row));
    }
  }
  writeFileSync(`${WORK}/bench.json`, JSON.stringify(results, null, 2));
}

prepareSources();
const mode = process.argv[2] ?? 'all';
if (mode === 'compat' || mode === 'all') compat();
if (mode === 'bench' || mode === 'all') bench();
