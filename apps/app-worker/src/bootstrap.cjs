'use strict';

// A trusted runtime artifact, loaded through Node's actual CommonJS loader.
// Embedded LoadEnvironment's initial require only supports built-in modules.
const { Socket } = require('node:net');
const fs = require('node:fs');
const { writeSync } = fs;
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

// App durability (packages/app-sdk/README.md, "Durability"). Node's own guard
// on descriptor fsync varies across releases (the pinned 24.20 denies the sync
// and callback forms but not the FileHandle forms; some releases deny none,
// newer ones deny all). Deny every form on the App's thread, so an App sees one
// behavior across runtime updates and does not come to depend on one release's gap.
// Durable data goes through state
// transactions or files.atomicReplace, which the kernel syncs before answering.
// This is policy, not containment: a worker thread keeps Node's own behavior.
async function denyFsync() {
  const denied = syscall => Object.assign(new Error(
    `${syscall} is not available to Apps; use chariox.files.atomicReplace or a state transaction for durable data`),
  { code: 'ERR_ACCESS_DENIED', syscall });
  const deniedCallback = syscall => function (fd, callback) {
    if (typeof callback !== 'function') {
      throw Object.assign(new TypeError('The "callback" argument must be of type function'), { code: 'ERR_INVALID_ARG_TYPE' });
    }
    process.nextTick(callback, denied(syscall));
  };
  fs.fsyncSync = () => { throw denied('fsync'); };
  fs.fdatasyncSync = () => { throw denied('fdatasync'); };
  fs.fsync = deniedCallback('fsync');
  fs.fdatasync = deniedCallback('fdatasync');
  // FileHandle is reachable only through an instance; this file is readable.
  const probe = await fs.promises.open(__filename, 'r');
  const fileHandlePrototype = Object.getPrototypeOf(probe);
  await probe.close();
  for (const [name, syscall] of [['sync', 'fsync'], ['datasync', 'fdatasync']]) {
    Object.defineProperty(fileHandlePrototype, name, {
      value: async function () { throw denied(syscall); }, writable: true, configurable: true,
    });
  }
  // ESM `import { fsyncSync } from 'node:fs'` must see the same functions.
  require('node:module').syncBuiltinESMExports();
}

// Node's permission model reaches a worker thread only through inherited
// options. On Node 24.20 a Worker given its own execArgv (even []) starts
// without --permission or with wider allow-lists, a NODE_OPTIONS in its env is
// parsed too, and module.register's hooks thread inherits --allow-worker and
// can start such a Worker. Before any App code: every Worker gets the
// launcher's permission flags without --allow-worker (so it cannot start
// Workers or hooks itself), App execArgv may only repeat launcher flags, env
// never carries NODE_OPTIONS, SHARE_ENV is refused, and so is module.register.
// This runs after App code has had a chance to replace built-ins, so it uses
// only references captured here.
function guardWorkerPermission() {
  const threads = require('node:worker_threads');
  const nodeModule = require('node:module');
  const { Worker: NativeWorker, SHARE_ENV } = threads;
  const { apply, construct, defineProperty } = Reflect;
  const { assign, freeze, getOwnPropertySymbols, getPrototypeOf, keys } = Object;
  const { isArray } = Array;
  const { includes } = Array.prototype;
  const hostEnv = process.env;
  const launcherFlags = freeze([...process.execArgv]);
  const workerFlags = freeze(launcherFlags.filter(flag => flag === '--permission' || flag === '--no-addons'
    || flag.startsWith('--allow-fs-read=') || flag.startsWith('--allow-fs-write=')));
  // Fail closed if the permission model is on but its flags are not visible.
  const unguarded = process.permission !== undefined && !workerFlags.includes('--permission');
  const failure = (message, code, Type = Error) => assign(new Type(message), { code });
  const denied = message => failure(message, 'ERR_ACCESS_DENIED');

  function workerOptions(options = {}) {
    if (unguarded) throw denied('App worker threads are unavailable in this runtime');
    // Own properties only: an execArgv or env on Object.prototype is ignored.
    const safe = { __proto__: null, ...options };
    const requested = safe.execArgv;
    if (requested) {
      if (!isArray(requested)) {
        throw failure('The "options.execArgv" property must be of type Array', 'ERR_INVALID_ARG_TYPE', TypeError);
      }
      const length = requested.length;
      for (let index = 0; index < length; index += 1) {
        const flag = requested[index];
        if (typeof flag !== 'string' || !apply(includes, launcherFlags, [flag])) {
          throw denied('App worker threads keep the App permission flags; execArgv may only repeat them');
        }
      }
    }
    safe.execArgv = workerFlags;
    if (safe.env === SHARE_ENV) throw denied('App worker threads cannot share the process environment');
    const source = safe.env ?? hostEnv;
    if (typeof source !== 'object') {
      throw failure('The "options.env" property must be of type object', 'ERR_INVALID_ARG_TYPE', TypeError);
    }
    const names = keys(source);
    const env = { __proto__: null };
    for (let index = 0; index < names.length; index += 1) {
      if (names[index] !== 'NODE_OPTIONS') env[names[index]] = `${source[names[index]]}`;
    }
    safe.env = env;
    return safe;
  }

  // A returned Worker, and the worker_threads diagnostics channel, both expose
  // the native thread handle, whose constructor is the one reachable way to
  // build a fresh handle that can start a thread: a handle faked with
  // Object.create has no native state and its startThread throws an
  // illegal-invocation TypeError. So neutralize that constructor on the shared
  // handle prototype now, before any App or migration code runs and could
  // capture the original. A reference an App reads later, from an instance or
  // the channel, is this neutralized function, and the original is unreachable.
  const seed = new NativeWorker('0', { eval: true, execArgv: workerFlags });
  const handlePrototype = (() => {
    const symbols = getOwnPropertySymbols(seed);
    for (let index = 0; index < symbols.length; index += 1) {
      if (symbols[index].description === 'kHandle') return getPrototypeOf(seed[symbols[index]]);
    }
    return null;
  })();
  seed.terminate();
  if (!handlePrototype || typeof handlePrototype.startThread !== 'function') {
    throw denied('App worker threads are unavailable in this runtime');
  }
  defineProperty(handlePrototype, 'constructor', {
    value: function Worker() { throw denied('App worker threads start only through node:worker_threads'); },
    writable: true, configurable: true,
  });

  // Not a Proxy: a Proxy keeps the native constructor as its target, which
  // util.inspect(worker, { showProxy: true }) formats and hands back through a
  // forwarded custom-inspect hook. A plain wrapper closes over the native
  // constructor instead; getPrototypeOf(Worker) is Function.prototype, and
  // neither the wrapper, its prototype nor an instance leads back to it.
  // Subclasses keep new.target, so `class extends Worker` still works.
  const Worker = function Worker(filename, options) {
    if (new.target === undefined) {
      throw failure("Class constructor Worker cannot be invoked without 'new'", 'ERR_CONSTRUCTION', TypeError);
    }
    return construct(NativeWorker, [filename, workerOptions(options)], new.target);
  };
  Worker.prototype = NativeWorker.prototype;
  defineProperty(NativeWorker.prototype, 'constructor', { value: Worker, writable: true, configurable: true });
  threads.Worker = Worker;
  // Node refuses register itself when --allow-worker is absent.
  nodeModule.register = function register() {
    throw denied('module.register is not available to Apps');
  };
  // ESM `import { Worker } from 'node:worker_threads'` must see the same functions.
  nodeModule.syncBuiltinESMExports();
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
    await denyFsync();
    guardWorkerPermission();
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
