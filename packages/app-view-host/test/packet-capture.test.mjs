// MP-08 / MP-09 / MP-10 / MP-11 — read root-owned evidence without privilege loss.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { assessPackets } from './packet-capture.mjs';

test('offline reader can assess a private capture and rejects packet drops', async () => {
  const evidence = await mkdtemp(join(tmpdir(), 'av1-pcap-'));
  try {
    // Empty little-endian Ethernet pcap: global header, no packet records.
    const header = Buffer.from('d4c3b2a1020004000000000000000000ffff000001000000', 'hex');
    await writeFile(join(evidence, 'receiver.pcap'), header);
    await writeFile(join(evidence, 'capture-process.json'), JSON.stringify({ state: 'exited', exitCode: 0 }));
    await writeFile(join(evidence, 'tcpdump.log'), '0 packets captured\n0 packets dropped by kernel\n');
    const result = await assessPackets(evidence, [], 'dst host 127.0.0.1');
    assert.equal(result.complete, true);
    assert.equal(result.nonLoopbackPackets, 0);
    assert.equal(result.strictReceiverPackets, 0);
    const ipv6 = Buffer.from('00000000000000000000000086dd600000000008114020010db800000000000000000000000120014860486000000000000000008888c00001bb00080000', 'hex');
    const record = Buffer.alloc(16);
    record.writeUInt32LE(2, 0); record.writeUInt32LE(ipv6.length, 8); record.writeUInt32LE(ipv6.length, 12);
    await writeFile(join(evidence, 'receiver.pcap'), Buffer.concat([header, record, ipv6]));
    assert.equal((await assessPackets(evidence, [], 'dst host 127.0.0.1')).nonLoopbackPackets, 1);
    const loopback = Buffer.from('00000000000000000000000008004500001c00000000401100007f0000017f000001c00022b800080000', 'hex');
    record.writeUInt32LE(loopback.length, 8); record.writeUInt32LE(loopback.length, 12);
    await writeFile(join(evidence, 'receiver.pcap'), Buffer.concat([header, record, loopback]));
    const filter = 'dst host 127.0.0.1 and udp dst port 8888';
    assert.equal((await assessPackets(evidence, [{ time: 1000, name: 'launch-frame-csp-control' }], filter)).strictReceiverPackets, 0);
    assert.equal((await assessPackets(evidence, [{ time: 1000, name: 'fetch-dpr1-strict' }], filter)).strictReceiverPackets, 1);
    await writeFile(join(evidence, 'tcpdump.log'), '1 packets dropped by kernel\n');
    assert.equal((await assessPackets(evidence, [], 'dst host 127.0.0.1')).dropped, 1);
  } finally { await rm(evidence, { recursive: true }); }
});
