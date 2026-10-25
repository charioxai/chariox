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
