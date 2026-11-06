// MP-08 / MP-09 / MP-10 / MP-11 — fail closed on real strace socket forms.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { processNetwork } from './process-network.mjs';

async function capture(lines, identified = true, complete = true) {
  const directory = await mkdtemp(join(tmpdir(), 'av1-syscalls-'));
  try {
    await writeFile(join(directory, 'pid-map.json'), JSON.stringify([{ processes: identified
      ? [{ pid: 9000, type: 'network.mojom.NetworkService', tasks: [9000, 9001] }] : [] }]));
    await writeFile(join(directory, 'syscall.9001'), [...lines, ...(complete ? ['9.0 +++ exited with 0 +++'] : [])].join('\n'));
    await processNetwork(directory);
    return JSON.parse(await readFile(join(directory, 'network.json'), 'utf8'));
  } finally { await rm(directory, { recursive: true }); }
}
const connect = (port, address = '127.0.0.1') => `1.0 connect(19<UDP:[0.0.0.0:21579]>, {sa_family=AF_INET, sin_port=htons(${port}), sin_addr=inet_addr("${address}")}, 16) = 0`;

test('a live, fully attributed loopback-only capture passes', async () => {
  const result = await capture([connect(8080), '2.0 sendto(19<UDP:[127.0.0.1:21579->127.0.0.1:8080]>, "probe", 5, 0, NULL, 0) = 5']);
  assert.equal(result.passed, true);
  assert.equal(result.outgoing.length, 2);
  assert(result.outgoing.every(row => row.pid === 9000 && row.tid === 9001));
});

test('connected DNS sends with no repeated sockaddr still fail the gate', async () => {
  const result = await capture([connect(53, '127.0.0.53'), '2.0 sendto(19<UDP:[0.0.0.0:21579]>, "question", 8, 0, NULL, 0) = 8']);
  assert.equal(result.passed, false);
  assert.equal(result.dnsQueries.length, 1);
  assert.equal(result.unknown.length, 0);
});

test('raw syscall findings retain the IPv6 route probe for the final assessment', async () => {
  const result = await capture(['1.0 connect(19<UDPv6:[993178997]>, {sa_family=AF_INET6, sin6_port=htons(443), sin6_flowinfo=htonl(0), inet_pton(AF_INET6, "2001:4860:4860::8888", &sin6_addr), sin6_scope_id=0}, 28) = 0']);
  assert.equal(result.passed, false);
  assert.equal(result.nonLoopback.length, 1);
  assert.equal(result.dnsQueries.length, 0);
});

test('unidentified processes and reused descriptors cannot silently pass', async () => {
  assert.equal((await capture([connect(8080)], false)).passed, false);
  const reused = await capture([connect(8080), '2.0 close(19<UDP:[0.0.0.0:21579]>) = 0', '3.0 sendto(19<UDP:[0.0.0.0:31000]>, "unknown", 7, 0, NULL, 0) = 7']);
  assert.equal(reused.passed, false);
  assert.equal(reused.unknown.length, 1);
  assert.equal((await capture([])).passed, false);
});

test('every destination in a sendmmsg batch is checked', async () => {
  const result = await capture(['1.0 sendmmsg(19<UDP:[0.0.0.0:21579]>, [{msg_hdr={msg_name={sa_family=AF_INET, sin_port=htons(8080), sin_addr=inet_addr("127.0.0.1")}, msg_iov=[{iov_base="control", iov_len=7}]}}, {msg_hdr={msg_name={sa_family=AF_INET, sin_port=htons(53), sin_addr=inet_addr("192.0.2.1")}, msg_iov=[{iov_base="question", iov_len=8}]}}], 2, 0) = 2']);
  assert.equal(result.passed, false);
  assert.equal(result.nonLoopback.length, 1);
  assert.equal(result.dnsQueries.length, 1);
});

test('an interrupted capture cannot pass', async () => {
  assert.equal((await capture([connect(8080)], true, false)).passed, false);
});
