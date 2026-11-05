// MP-08 / MP-09 / MP-10 / MP-11 — owner-authorized route probe, no wider grant.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { processNetwork } from './process-network.mjs';
import { assessResults } from './drill-result.mjs';

const socket = '1.0 socket(AF_INET6, SOCK_DGRAM|SOCK_CLOEXEC, IPPROTO_IP) = 19<UDPv6:[993178997]>';
const connect = '2.0 connect(19<UDPv6:[993178997]>, {sa_family=AF_INET6, sin6_port=htons(443), sin6_flowinfo=htonl(0), inet_pton(AF_INET6, "2001:4860:4860::8888", &sin6_addr), sin6_scope_id=0}, 28) = 0';
const packets = { complete: true, dropped: 0, nonLoopbackPackets: 0, strictReceiverPackets: 0 };
async function assess({ lines = [socket, connect], type = 'network.mojom.NetworkService', complete = true, packetCapture = packets } = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'av1-route-'));
  try {
    await writeFile(join(directory, 'pid-map.json'), JSON.stringify([{ processes: type ? [{ pid: 9000, type, tasks: [9000, 9001] }] : [] }]));
    await writeFile(join(directory, 'syscall.9001'), [...lines, ...(complete ? ['9.0 +++ exited with 0 +++'] : [])].join('\n'));
    const network = await processNetwork(directory);
    const results = {
      isolationNetwork: network,
      attacks: [{ name: 'fetch', blocked: true, redCapable: true, framePolicyPresent: true,
        runs: [{ processNetwork: network }, { processNetwork: network }] }],
      references: Array.from({ length: 4 }, () => ({ rendered: true, processNetwork: network })),
      launch: { passed: true, rows: [{ kind: 'idle', control: false, network }, { kind: 'literal', control: true, network }] },
      isolation: { passed: true }, sourceBinding: { passed: true }, rejectedBeforeExecution: true,
      packetCapture,
    };
    assessResults(results);
    return results;
  } finally { await rm(directory, { recursive: true }); }
}

test('shared final assessment accepts exactly the recorded Network Service route probe and audits it', async () => {
  const result = await assess();
  assert.equal(result.containmentPassed, true);
  assert.equal(result.networkExemptions.length, 1);
  const exemption = result.networkExemptions[0];
  assert.equal(exemption.name, 'chromium-network-service-ipv6-route-probe');
  assert.equal(exemption.pid, 9000);
  assert.equal(exemption.tid, 9001);
  assert.equal(exemption.address, '2001:4860:4860::8888');
  assert.equal(exemption.port, 443);
  assert.equal(exemption.result, '0');
  assert.equal(exemption.operation, 'connect');
});

test('the same connect-only probe may return EADDRNOTAVAIL', async () => {
  assert.equal((await assess({ lines: [socket, connect.replace('= 0', '= -1 EADDRNOTAVAIL (Cannot assign requested address)')] })).containmentPassed, true);
});

const variants = [
  ['other IPv6 destination', { lines: [socket, connect.replace('2001:4860:4860::8888', '2001:4860:4860::8844')] }],
  ['other port', { lines: [socket, connect.replace('htons(443)', 'htons(444)')] }],
  ['TCP', { lines: [socket.replace('SOCK_DGRAM', 'SOCK_STREAM').replace('UDPv6', 'TCPv6'), connect.replace('UDPv6', 'TCPv6')] }],
  ['missing socket creation', { lines: [connect] }],
  ['renderer-owned probe', { type: 'renderer' }],
  ['browser-owned probe', { type: 'browser' }],
  ['GPU-owned probe', { type: 'GPU' }],
  ['unidentified sender', { type: null }],
  ['incomplete trace', { complete: false }],
  ['DNS send', { lines: [socket, connect, '3.0 sendto(20<UDP:[0.0.0.0:31000]>, "question", 8, 0, {sa_family=AF_INET, sin_port=htons(53), sin_addr=inet_addr("127.0.0.53")}, 16) = 8'] }],
  ['second destination', { lines: [socket, connect, connect.replace('2.0', '3.0').replace('2001:4860:4860::8888', '2001:db8::1')] }],
  ['packet on the wire', { packetCapture: { ...packets, nonLoopbackPackets: 1 } }],
  ['strict receiver packet', { packetCapture: { ...packets, strictReceiverPackets: 1 } }],
  ['missing packet capture', { packetCapture: null }],
  ['incomplete packet capture', { packetCapture: { ...packets, complete: false } }],
  ['dropped capture packet', { packetCapture: { ...packets, dropped: 1 } }],
];
for (const operation of ['sendto', 'sendmsg', 'sendmmsg', 'write', 'writev', 'sendfile', 'splice']) {
  const line = operation === 'splice' ? '3.0 splice(20, NULL, 19, NULL, 5, 0) = 5'
    : `3.0 ${operation}(19<UDPv6:[993178997]>, "probe", 5, 0, NULL, 0) = 5`;
  variants.push([`${operation} on the probe socket`, { lines: [socket, connect, line] }]);
}
variants.push(['send after reconnecting the probe to loopback', { lines: [socket, connect,
  connect.replace('2.0', '3.0').replace('2001:4860:4860::8888', '::1'), '4.0 write(19, "probe", 5) = 5'] }]);
for (const duplicate of ['dup(19) = 20', 'dup2(19, 20) = 20', 'dup3(19, 20, O_CLOEXEC) = 20', 'fcntl(19, F_DUPFD_CLOEXEC, 0) = 20']) {
  variants.push([`write through ${duplicate}`, { lines: [socket, connect, `3.0 ${duplicate}`, '4.0 write(20, "probe", 5) = 5'] }]);
}
for (const [name, options] of variants) test(`shared final assessment rejects ${name}`, async () => {
  const result = await assess(options);
  assert.equal(result.containmentPassed, false);
  assert.deepEqual(result.networkExemptions ?? [], []);
});
