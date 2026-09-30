'use strict';

// A trusted runtime artifact, loaded through Node's actual CommonJS loader.
// Embedded LoadEnvironment's initial require only supports built-in modules.
const { Socket } = require('node:net');
const { writeSync } = require('node:fs');
const { pathToFileURL } = require('node:url');
const { configuration } = require('./bootstrap-config.cjs');
const exit = process.exit.bind(process);
const later = setTimeout;
const cancelTimer = clearTimeout;
let started = false;

// Node watches a macOS directory through FSEvents. The worker's Seatbelt
// policy denies that service because fseventsd reports deleted and renamed
// paths outside the worker's roots (launcher.md), so libuv fails the watch
// with EMFILE. Directory watches poll instead; file watches keep kqueue.
const WATCH_INTERVAL_MS = 250;
const WATCH_ENTRY_LIMIT = 4096;

function installDirectoryWatch(platform) {
  if (platform !== 'darwin') return;
  const fs = require('node:fs');
  const path = require('node:path');
  const { EventEmitter, on } = require('node:events');
  const { fileURLToPath } = require('node:url');
  const nativeWatch = fs.watch;
  const nativePromisesWatch = fs.promises.watch;
  const directory = target => { try { return fs.statSync(target).isDirectory(); } catch { return false; } };
  const text = target => (target instanceof URL ? fileURLToPath(target) : String(target));
  const failure = (message, code, extra) => Object.assign(new Error(message), { code, ...extra });

  // Names relative to the root (recursively when asked) with change stamps.
  function scan(root, recursive) {
    const entries = new Map();
    const visit = relative => {
      let names;
      try { names = fs.readdirSync(path.join(root, relative)); }
      catch (error) { if (!relative) throw error; return; }
      for (const name of names) {
        const key = relative ? path.join(relative, name) : name;
        let stats;
        try { stats = fs.lstatSync(path.join(root, key)); } catch { continue; }
        if (entries.size === WATCH_ENTRY_LIMIT) {
          throw failure(`ENOSPC: directory watch limit of ${WATCH_ENTRY_LIMIT} entries reached, watch '${root}'`,
            'ENOSPC', { syscall: 'watch', path: root });
        }
        entries.set(key, [stats.ino, stats.isDirectory(), stats.size, stats.mtimeMs, stats.ctimeMs]);
        if (recursive && stats.isDirectory()) visit(key);
      }
    };
    visit('');
    return entries;
  }

  class DirectoryWatcher extends EventEmitter {
    #root; #recursive; #encoding; #persistent; #entries; #timer; #closed = false;
    constructor(root, { persistent = true, recursive = false, encoding = 'utf8', signal } = {}) {
      super();
      if (encoding !== 'buffer' && !Buffer.isEncoding(encoding)) {
        throw failure(`The argument 'encoding' is invalid encoding. Received '${encoding}'`, 'ERR_INVALID_ARG_VALUE');
      }
      this.#root = root; this.#recursive = Boolean(recursive); this.#encoding = encoding;
      this.#persistent = persistent !== false;
      this.#entries = scan(root, this.#recursive);
      this.#schedule();
      if (signal?.aborted) process.nextTick(() => this.close());
      else signal?.addEventListener('abort', () => this.close(), { once: true });
    }
    #schedule() {
      this.#timer = later(() => this.#poll(), WATCH_INTERVAL_MS);
      if (!this.#persistent) this.#timer.unref();
    }
    #poll() {
      let next;
      try { next = scan(this.#root, this.#recursive); }
      catch (error) {
        if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') {
          this.#closed = true;
          this.emit('error', error);
          return;
        }
        next = new Map();
      }
      const previous = this.#entries;
      this.#entries = next;
      for (const [name, [ino, isDirectory, ...stamps]] of previous) {
        const now = next.get(name);
        if (!now || now[0] !== ino || now[1] !== isDirectory) this.#report('rename', name);
        else if (!isDirectory && stamps.some((stamp, index) => now[index + 2] !== stamp)) this.#report('change', name);
      }
      for (const name of next.keys()) if (!previous.has(name)) this.#report('rename', name);
      if (!this.#closed) this.#schedule();
    }
    #report(eventType, name) {
      if (this.#closed) return;
      const filename = this.#encoding === 'buffer' ? Buffer.from(name) : Buffer.from(name).toString(this.#encoding);
      this.emit('change', eventType, filename);
    }
    close() {
      if (this.#closed) return;
      this.#closed = true;
      cancelTimer(this.#timer);
      process.nextTick(() => this.emit('close'));
    }
    ref() { this.#persistent = true; this.#timer?.ref(); return this; }
    unref() { this.#persistent = false; this.#timer?.unref(); return this; }
  }

  fs.watch = function watch(target, options, listener) {
    if (!directory(target)) return Reflect.apply(nativeWatch, fs, arguments);
    if (typeof options === 'function') { listener = options; options = undefined; }
    if (typeof options === 'string') options = { encoding: options };
    const watcher = new DirectoryWatcher(text(target), options ?? {});
    if (listener) watcher.on('change', listener);
    return watcher;
  };
  // events.on ends with an AbortError on abort and throws a watcher error.
  // maxQueue/overflow are not applied: the queue holds what the App has not read.
  fs.promises.watch = async function* watch(target, options = {}) {
    if (!directory(target)) { yield* nativePromisesWatch(target, options); return; }
    if (typeof options === 'string') options = { encoding: options };
    const { signal, ...rest } = options ?? {};
    const watcher = new DirectoryWatcher(text(target), rest);
    try {
      for await (const [eventType, filename] of on(watcher, 'change', { signal })) yield { eventType, filename };
    } finally { watcher.close(); }
  };
  // ESM `import { watch } from 'node:fs'` must see the same functions.
  require('node:module').syncBuiltinESMExports();
}

function start(input) {
  let sdk;
  let transport;
  let timer;
  let draining = false;
  let finished = false;
  const finish = code => {
    if (finished) return;
    finished = true;
    cancelTimer(timer);
    try { sdk?.close(); transport?.close(); } catch { /* process exit closes FD3 */ }
    if (code) {
      try { writeSync(2, `app_worker_bootstrap_failed:${code}\n`); } catch { /* bounded diagnostic only */ }
    }
    exit(code);
  };
  if (started) { finish(130); return; }
  started = true;
  process.on('uncaughtException', () => finish(134));
  process.on('unhandledRejection', () => finish(134));
  let config;
  try { config = configuration(input, process.env, __dirname); }
  catch { finish(130); return; }
  const arm = timeoutMs => { cancelTimer(timer); timer = later(() => finish(132), timeoutMs); };
  arm(config.migrations.length ? config.migrationTimeoutMs : config.startupTimeoutMs);

  async function load() {
    // Runtime packaging pins this complete SDK source graph. Never resolve an
    // SDK from the App, cwd, NODE_PATH, an installation script, or a URL.
    const { createAppSdk } = require('./sdk/src/index.js');
    const { streamTransport } = require('./sdk/src/transport.js');
    const stream = new Socket({ fd: 3, readable: true, writable: true });
    const channel = streamTransport(stream, { frameTimeoutMs: 15000 });
    let registered = false;
    let shutdown;
    transport = {
      subscribe(onMessage, onClose) {
        return channel.subscribe(message => {
          // Normal operation dispatch starts after registration. Kernel policy
          // also withholds operations until it accepts worker.ready.
          if (message.kind === 'request' && !registered) { finish(133); return; }
          if (message.kind === 'request' && message.method === 'lifecycle.dispatch'
            && message.params?.event === 'shutdown' && shutdown === undefined) shutdown = message.id;
          onMessage(message);
        }, reason => {
          onClose(reason);
          if (!draining) finish(133);
        });
      },
      send(message) {
        channel.send(message);
        if (shutdown !== undefined && message.kind === 'response' && message.id === shutdown) {
          draining = true;
          cancelTimer(timer);
          timer = later(() => finish(135), 1000);
          // net.Socket.end's callback follows all queued writes. Destroying the
          // SDK immediately would lose the lifecycle response under backpressure.
          stream.end(() => finish(Object.hasOwn(message, 'error') ? 135 : 0));
        }
      },
      close: () => channel.close(),
    };
    sdk = createAppSdk({ transport, generation: config.generation,
      paths: config.paths, declarations: config.declarations });
    // Preserve Node's Web value objects while routing every global Fetch call
    // through the actual worker SDK channel before loading any App module.
    Object.defineProperty(globalThis, 'fetch', { value: sdk.http.fetch, writable: false, configurable: false });
    installDirectoryWatch(process.platform);
    const { ready, close, migration, migrationStep, ...api } = sdk;
    void ready; void close;
    if (config.migrations.length) {
      // Pending data migrations run in order before any App runtime module.
      // The kernel admits only state calls and migration.step until readiness.
      try {
        for (const step of config.migrations) {
          const module = await import(pathToFileURL(step.entry).href);
          if (typeof module.default !== 'function') throw new Error('migration');
          await module.default(Object.freeze(migration(step)));
          if (await migrationStep(step.to) !== null) throw new Error('migration');
        }
      } catch { finish(136); return; }
      arm(config.startupTimeoutMs);
    }
    const app = await import(pathToFileURL(config.entry).href);
    if (typeof app.default !== 'function') { finish(131); return; }
    await app.default(Object.freeze(api));
    registered = true;
    if (await sdk.ready({ timeoutMs: config.startupTimeoutMs }) !== null) { finish(131); return; }
    if (!draining) cancelTimer(timer);
    // FD3 keeps this process alive. Only kernel lifecycle shutdown, channel
    // loss, fatal errors or the outer supervisor end its lifetime.
  }
  void load().catch(() => { if (!draining) finish(131); });
}

module.exports = Object.freeze({ start, installDirectoryWatch });
