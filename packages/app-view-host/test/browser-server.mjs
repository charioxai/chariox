import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createSocket } from 'node:dgram';
import { spawn } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { openSync, closeSync } from 'node:fs';
import { join, resolve, sep, extname } from 'node:path';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { controlHost } from './control-host.mjs';
import { routeProbeFilter, assessPackets } from './packet-capture.mjs';

export async function browserServer({ evidence, webDist, build }) {
  const hostPath = '/assets/app-view-host/host.html';
  const clientPath = '/assets/app-view-host/client.mjs';
  // The strict renderer is read from Cloud's completed web build, never rebuilt
  // here. Only deliberately weakened controls are compiled in this process.
  const hostDocument = await readFile(join(webDist, hostPath), 'utf8');
  const controls = new Map();
  for (const name of ['csp', 'sandbox', 'bootstrap', 'all', 'wasm', 'source']) controls.set(name, await controlHost(build, name));
  const sockets = new Set(), receivers = [], hits = [], connections = [], packets = [];
  let boundary = null, phase = 'initialization', capture, captureLog = '';
  let closed = false;
  const phases = [{ name: phase, time: Date.now() }];
  function track(server) {
    server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
  }
  async function receiver(control) {
    const http = createServer((request, response) => {
      hits.push({ time: Date.now(), phase, control, url: request.url, method: request.method });
      response.setHeader('Access-Control-Allow-Origin', '*');
      response.setHeader('Content-Type', request.url.endsWith('.js') ? 'text/javascript' : request.url.endsWith('.css') ? 'text/css' : 'text/html');
      response.end(request.url.endsWith('.js') ? 'globalThis.receiverScriptRan=true;' : request.url.endsWith('.css') ? 'body{color:blue}' : '<!doctype html>receiver');
    });
    http.on('upgrade', (request, socket) => {
      hits.push({ time: Date.now(), phase, control, url: request.url, method: 'UPGRADE' });
      socket.end('HTTP/1.1 400 Bad Request\r\n\r\n');
    });
    track(http);
    http.on('connection', socket => {
      const item = { time: Date.now(), phase, control, remotePort: socket.remotePort, bytes: 0 };
      connections.push(item); socket.on('data', bytes => { item.bytes += bytes.length; });
    });
    const udp = createSocket('udp4');
    udp.on('message', (bytes, remote) => packets.push({ time: Date.now(), phase, control, size: bytes.length, remotePort: remote.port }));
    await new Promise(r => http.listen(0, '127.0.0.1', r));
    await new Promise(r => udp.bind(0, '127.0.0.1', r));
    const item = { control, http, udp, tcpPort: http.address().port, udpPort: udp.address().port };
    receivers.push(item);
    return { url: `http://127.0.0.1:${item.tcpPort}`, udpPort: item.udpPort };
  }
  const controlReceiver = await receiver(true), strictReceiver = await receiver(false);
  const server = createServer(async (request, response) => {
    try {
      const path = new URL(request.url, 'http://localhost').pathname;
      response.setHeader('Cache-Control', 'no-store');
      if (!boundary) response.setHeader('X-DNS-Prefetch-Control', 'off');
      if (path === hostPath) {
        response.setHeader('Content-Type', 'text/html');
        const html = boundary ? controls.get(boundary) : hostDocument;
        // Match Cloud's dedicated static response, including inherited HTTP CSP.
        // Its real serveWebAsset entry is separately exercised by the Cloud test.
        const policy = /<meta http-equiv="Content-Security-Policy" content="([^"]+)">/.exec(html)?.[1];
        if (policy) response.setHeader('Content-Security-Policy', policy + "; frame-ancestors 'self'");
        response.end(html); return;
      }
      if (path === '/probe.html') {
        response.setHeader('Content-Type', 'text/html');
        response.end('<!doctype html><title>AV1 containment probe</title>'); return;
      }
      if (path === '/av1-worker/sw.js') {
        response.setHeader('Content-Type', 'text/javascript');
        response.end('self.addEventListener("install",()=>self.skipWaiting());'); return;
      }
      if (path.startsWith('/auth/')) {
        response.writeHead(401, { 'Content-Type': 'application/json' }); response.end('{}'); return;
      }
      const file = resolve(webDist, '.' + (path === '/' ? '/index.html' : path));
      if (!file.startsWith(resolve(webDist) + sep)) { response.writeHead(404); response.end(); return; }
      const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.woff2': 'font/woff2', '.svg': 'image/svg+xml' };
      response.setHeader('Content-Type', types[extname(file)] ?? 'application/octet-stream');
      response.end(await readFile(file));
    } catch { response.writeHead(404); response.end(); }
  });
  track(server);
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const origin = `http://127.0.0.1:${server.address().port}`;
  async function open(page, packed, builtApp = false) {
    await page.goto(origin + (builtApp ? '/' : '/probe.html'));
    await page.evaluate(async ({ packed, hostPath, clientPath }) => {
      const { mountAppView } = await import(clientPath);
      const shell = document.createElement('section');
      shell.id = 'av1-fixture'; shell.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:white;display:flex;flex-direction:column';
      shell.innerHTML = '<div id="conversation" data-private="adjacent-private-canary">AV1 adjacent-chrome canary (no session attached)</div><button id="av1-open">Open reference App</button><div id="av1-mount" style="flex:1;min-height:0"></div>';
      document.body.append(shell);
      window.probeStarts = []; window.probeResults = [];
      addEventListener('message', event => {
        const host = document.querySelector('#av1-mount iframe');
        if (event.source !== host?.contentWindow?.frames[0]) return;
        if (event.data?.type === 'av1.probe-start') window.probeStarts.push(event.data.name);
        if (event.data?.type === 'av1.probe-result') window.probeResults.push(event.data.probe);
      });
      const button = document.querySelector('#av1-open');
      button.onclick = () => {
        window.view = mountAppView(document.querySelector('#av1-mount'), { hostUrl: hostPath, ...packed, call: async () => { throw Object.assign(new Error('The AV1 reference Apps have no backend'), { code: 'NO_BACKEND' }); } });
        window.viewReady = window.view.ready;
      };
    }, { packed, hostPath, clientPath });
    await page.locator('#av1-open').click();
    await page.evaluate(() => window.viewReady);
  }
  async function frame(page) {
    const host = page.frames().find(f => f.url() === origin + hostPath);
    assert(host, 'trusted host loaded from built Cloud assets');
    await host.waitForSelector('iframe');
    return host.childFrames()[0];
  }
  return {
    origin, hostDocument, controlReceiver, strictReceiver, receivers, hits, connections, packets, open, frame,
    setBoundary(value) { assert(value === null || controls.has(value)); boundary = value; },
    setPhase(value) { phase = value; phases.push({ name: value, time: Date.now() }); },
    async assessPackets() {
      const receiver = receivers.find(receiver => !receiver.control);
      return assessPackets(evidence, phases, `dst host 127.0.0.1 and (tcp dst port ${receiver.tcpPort} or udp dst port ${receiver.udpPort})`);
    },
    async capture() {
      const ports = receivers.flatMap(r => [`tcp port ${r.tcpPort}`, `udp port ${r.udpPort}`]).join(' or ');
      // Capture only this drill's DNS names, never other lanes' resolver traffic.
      const dns = 'udp dst port 53 and udp[21:4] = 0x61707076 and udp[25:4] = 0x6965772d and udp[29:4] = 0x646e732d';
      const filter = `(dst host 127.0.0.1 and (${ports})) or (${dns}) or (${routeProbeFilter})`;
      const output = openSync(join(evidence, 'receiver.pcap'), 'wx');
      try {
        capture = spawn('tcpdump', ['-Z', 'root', '-U', '-n', '-i', 'any', '-s', '0', '-w', '-', filter], { stdio: ['ignore', output, 'pipe'] });
      } finally { closeSync(output); }
      const started = once(capture, 'spawn');
      capture.stderr.on('data', chunk => { captureLog += chunk; });
      await started; await delay(300);
      assert.equal(capture.exitCode, null, 'tcpdump is running');
      await writeFile(join(evidence, 'capture-filter.txt'), filter + '\n');
      await writeFile(join(evidence, 'capture-process.json'), JSON.stringify({ pid: capture.pid, state: 'running' }));
      await writeFile(join(evidence, 'receiver-ports.json'), JSON.stringify(receivers.map(({ control, tcpPort, udpPort }) => ({ control, tcpPort, udpPort })), null, 2));
    },
    async close() {
      if (closed) return;
      closed = true;
      if (capture && Number.isSafeInteger(capture.pid) && capture.pid > 1 && capture.exitCode === null) {
        const exited = once(capture, 'exit'); capture.kill('SIGINT'); await exited;
      }
      if (capture) await writeFile(join(evidence, 'capture-process.json'), JSON.stringify({ pid: capture.pid, exitCode: capture.exitCode, signal: capture.signalCode, state: 'exited' }));
      for (const socket of sockets) socket.destroy();
      await new Promise(r => server.close(r));
      for (const receiver of receivers) { await new Promise(r => receiver.http.close(r)); receiver.udp.close(); }
      await writeFile(join(evidence, 'tcpdump.log'), captureLog);
    },
  };
}
