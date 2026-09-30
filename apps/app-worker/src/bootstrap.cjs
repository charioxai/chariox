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

module.exports = Object.freeze({ start });
