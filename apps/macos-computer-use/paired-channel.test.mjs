import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
const read = file => readFileSync(new URL(file, import.meta.url), 'utf8');
test('kernel cancellation fences an in-flight paired request without waiting for channel lock', () => {
  const source = read('../kernel/src/runtime/kernel_browser_host/mac_helper.rs');
  assert.match(source, /RequestFence::watch/);
  assert.match(source, /cancel.requested\(\)[\s\S]*link.fence\(\)/);
  const access = read('../kernel/src/runtime/kernel_browser_host/access.rs');
  assert.match(access, /fence_computer_actions\(\)/);
});
test('paired native fences consult the kernel connection at the final mutation boundary', () => {
  const native = read('./Native.swift');
  const destination = native.slice(native.indexOf('struct NativeDestinationFence'), native.indexOf('struct MacSource'));
  assert.match(destination, /occlusion\(request, geometry.location, geometry.windowBounds\)[\s\S]*if checkAuthority \{ try authority\(\) \}/);
  assert.match(native, /occlusion: \{ try checkWindowOcclusion/);
  assert.match(native, /windowBounds: windowBounds[\s\S]*authority: authority/);
  const channel = read('./PairedSession.swift');
  assert.match(channel, /MSG_PEEK \| MSG_DONTWAIT/);
  assert.match(channel, /grant-revoked/);
});
