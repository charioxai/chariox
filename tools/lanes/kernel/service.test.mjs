// MP-08 / MP-10 / MP-11: boot recovery and slice placement without touching host services.
import test from 'node:test';
import assert from 'node:assert/strict';
import { serviceUnit, sliceInvocation } from './service.mjs';
test('start enters lanes.slice when available and does not nest within the slice', () => {
  const input = { sliceExists: true, cgroup: '0::/user.slice/session.scope', node: '/usr/bin/node', script: '/checkout/tools/lanes/kernel/lanes-kernel.mjs', args: ['start'] };
  assert.deepEqual(sliceInvocation(input), ['systemd-run', '--quiet', '--slice=lanes.slice', '--scope', '--', input.node, input.script, 'start']);
  assert.equal(sliceInvocation({ ...input, cgroup: '0::/lanes.slice/chariox-lanes.service' }), undefined);
  assert.equal(sliceInvocation({ ...input, sliceExists: false }), undefined);
});
test('boot service starts and stops the exact state in lanes.slice at multi-user.target', () => {
  const unit = serviceUnit({ node: '/usr/bin/node', script: '/checkout/tools/lanes/kernel/lanes-kernel.mjs', state: '/own/state' });
  for (const expected of ['Slice=lanes.slice', 'WantedBy=multi-user.target', 'Type=oneshot', 'RemainAfterExit=yes', 'Restart=on-failure', 'LANES_KERNEL_STATE=/own/state', 'lanes-kernel.mjs" start', 'lanes-kernel.mjs" stop']) assert.ok(unit.includes(expected), expected);
  assert.throws(() => serviceUnit({ node: '/usr/bin/node', script: '/unsafe\npath', state: '/own/state' }), /path/);
});

test('the boot unit is shipped as an inspectable template in tools/lanes', async () => {
  const { readFile } = await import('node:fs/promises');
  const template = await readFile(new URL('./chariox-lanes.service', import.meta.url), 'utf8');
  assert.match(template, /Slice=lanes\.slice/);
  assert.match(template, /WantedBy=multi-user\.target/);
  assert.match(template, /ExecStart="@NODE@" "@SCRIPT@" start/);
});

test('the unit keeps the other lanes running when the kernel OOM-kills one process, and the slice caps the lanes below the host memory', async () => {
  const { readFile } = await import('node:fs/promises');
  const unit = serviceUnit({ node: '/usr/bin/node', script: '/checkout/tools/lanes/kernel/lanes-kernel.mjs', state: '/own/state' });
  assert.match(unit, /^OOMPolicy=continue$/m, 'systemd must not stop the whole service for one OOM kill (04:38Z 10-26: all nine lanes)');
  const slice = await readFile(new URL('./lanes.slice', import.meta.url), 'utf8');
  assert.match(slice, /^MemoryHigh=88G$/m);
  assert.match(slice, /^MemoryMax=96G$/m);
});
