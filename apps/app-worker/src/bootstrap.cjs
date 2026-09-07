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
  timer = later(() => finish(132), config.startupTimeoutMs);

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
    const { ready, close, ...api } = sdk;
    void ready; void close;
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
