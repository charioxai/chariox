import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { networkInterfaces } from 'node:os';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { browserProfile } from './browser-profile.mjs';
import { processNetwork } from './process-network.mjs';
import { clientViewChromiumArgs } from '../chromium-launch.mjs';

// Independent launch controls: same restricted profile, remove only the
// resolver mapping. The non-loopback endpoint belongs to this test process.
export async function launchCases({ chromium, launchOptions, evidence, server, pack }) {
  const address = Object.values(networkInterfaces()).flat().find(item => item.family === 'IPv4' && !item.internal)?.address;
  assert(address, 'a local non-loopback IPv4 address is required for the capture control');
  const hits = [], sockets = new Set(), rows = [];
  let phase = '';
  const receiver = createServer((request, response) => {
    hits.push({ phase, url: request.url });
    response.setHeader('Access-Control-Allow-Origin', '*'); response.end('owned launch control');
  });
  receiver.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
  await new Promise(resolve => receiver.listen(0, address, resolve));
  const endpoint = `http://${address}:${receiver.address().port}`;
  try {
    for (const kind of ['idle', 'literal', 'dns', 'frame-csp']) for (const control of (kind === 'idle' ? [false] : [true, false])) {
      phase = `launch-${kind}-${control ? 'control' : 'strict'}`;
      const directory = join(evidence, 'process-tree', phase);
      const args = clientViewChromiumArgs.filter(arg => !(control && ['literal', 'dns'].includes(kind) && arg.startsWith('--host-resolver-rules=')));
      let profile;
      const row = { kind, control, phase, endpoint, before: hits.length };
      try {
        profile = await browserProfile(chromium, { ...launchOptions, clientView: true,
          args: [...launchOptions.args, ...args, `--log-net-log=${join(evidence, 'netlog', phase + '.json')}`], captureDirectory: directory });
        const page = await profile.context.newPage();
        if (kind === 'literal') row.result = await page.evaluate(url => fetch(url).then(r => r.text(), e => e.name), endpoint + '/' + phase);
        if (kind === 'dns') row.result = await page.evaluate(url => fetch(url).then(() => 'unexpected', e => e.name), `http://appview-dns-${phase}.invalid/`);
        if (kind === 'frame-csp') {
          server.setBoundary(control ? 'csp' : null); server.setPhase(phase);
          row.beforeFrameHits = server.hits.length;
          await server.open(page, await pack(`fetch('${server.strictReceiver.url}/${phase}').catch(()=>{});`));
        }
        await delay(1300); await profile.snapshot('probe-complete');
        row.hits = hits.slice(row.before);
        if (kind === 'frame-csp') row.frameHits = server.hits.slice(row.beforeFrameHits);
      } finally { await profile?.close(); }
      row.network = await processNetwork(directory);
      const network = JSON.parse(await readFile(join(directory, 'network.json'), 'utf8'));
      row.ownedEndpointCalls = network.outgoing.filter(item => item.address === address && item.port === receiver.address().port).length;
      row.syntheticDnsCalls = network.dnsQueries.filter(item => item.line.includes('appview-dns-launch-dns-control')).length;
      rows.push(row);
    }
  } finally {
    server.setBoundary(null);
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => receiver.close(resolve));
  }
  const find = (kind, control) => rows.find(row => row.kind === kind && row.control === control);
  const passed = find('literal', true).hits.length > 0 && find('literal', false).hits.length === 0
    && find('literal', true).ownedEndpointCalls > 0 && find('literal', false).ownedEndpointCalls === 0
    && find('dns', true).syntheticDnsCalls > 0 && find('dns', false).network.dnsQueries === 0
    && find('frame-csp', true).frameHits.length > 0 && find('frame-csp', false).frameHits.length === 0;
  return { passed, scope: 'launch and frame controls independently; idle browser also traced without App content', rows };
}
