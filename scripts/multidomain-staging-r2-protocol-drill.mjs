// Credential-free round-2 protocol drill. No kernel boot, account state or relay.
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { LOCAL_DAEMON_PROTOCOL_VERSION } from '../packages/kernel-client/dist/kernel-types.js';
import { kernelBrowserRequest, kernelBrowserMinimumProtocolVersion, userDomainAccessMinimumProtocolVersion } from '../packages/kernel-client/dist/ipc-kernel-browser-requests.js';
import { attachBrowserMirror, browserMirrorMinimumProtocolVersion } from '../packages/kernel-client/dist/browser-mirror.js';
import { userDomainAccessMinimumProtocol } from '../packages/kernel-client/dist/user-domain-access.js';
import { notesMinimumProtocolVersion } from '../packages/kernel-client/dist/notes.js';
import { visibleRegionCaptureMinimumProtocolVersion } from '../packages/kernel-client/dist/ipc-screenshot-requests.js';
import { userAppViewsMinimumProtocolVersion } from '../packages/kernel-client/dist/ipc-app-requests.js';

const [binary, output] = process.argv.slice(2);
assert(binary && output, 'Usage: node scripts/multidomain-staging-r2-protocol-drill.mjs KERNEL_BINARY EVIDENCE_JSON');
const repo = path.resolve(import.meta.dirname, '..');
assert(!path.resolve(output).startsWith(repo + path.sep), 'Evidence must live outside the source checkout');
assert.equal(execFileSync(binary, ['--print-local-daemon-protocol-version'], { encoding: 'utf8' }).trim(), '443');
for (const value of [LOCAL_DAEMON_PROTOCOL_VERSION, kernelBrowserMinimumProtocolVersion,
  userDomainAccessMinimumProtocolVersion, browserMirrorMinimumProtocolVersion,
  userDomainAccessMinimumProtocol, notesMinimumProtocolVersion,
  visibleRegionCaptureMinimumProtocolVersion, userAppViewsMinimumProtocolVersion]) assert.equal(value, 443);
const bytes = await readFile(new URL('../apps/kernel/src/local/api/tests/protocol_shapes/staging-union-443.json', import.meta.url));
assert.equal(createHash('sha256').update(bytes).digest('hex'), '9a0ab6f6a986b63ab591b3f0563fac81b8fbf10d8c366ebeabd287fa678d7bb2');
const fixture = JSON.parse(bytes);
assert.equal(fixture.local_daemon_protocol_version, 443);
assert.equal(fixture.relay_peer_protocol_version, 86);
for (const request of fixture.requests) assert.deepEqual(kernelBrowserRequest(request.KernelBrowser.command), request);
let requests = 0;
for (const protocolVersion of [0, 427, 432, 433, 434, 442, NaN]) {
  await assert.rejects(attachBrowserMirror({ protocolVersion, request: async () => { requests++; assert.fail('Older kernels must not receive mirror requests'); } }, {},
    { tab_id: 't', generation: 1, device_scale_factor: 1 }, () => {}), /protocol 443/);
}
assert.equal(requests, 0);
const result = { status: 'PASS', local: 443, peer: 86, minima: 8,
  serialized_commands: fixture.requests.length, old_kernel_guards: 7, dispatched_requests: requests,
  fixture_sha256: createHash('sha256').update(bytes).digest('hex') };
await writeFile(output, JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(result));
