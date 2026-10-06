// MP-08 / MP-11: fail-first host lifecycle contracts.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { LinuxOwnedDesktop, desktopEnvironment, desktopBusAddress } from './linux-owned-desktop.mjs';
import { validOwnedPid, processIdentity, descendants, signalOwned, settleOwned } from './linux-owned-process.mjs';

test('MP-11 rejects root and inherited desktop authority', () => {
  assert.throws(() => desktopEnvironment({ DISPLAY: ':0' }, '/tmp/x', 0), /non-root/);
  const env = desktopEnvironment({ PATH: '/usr/bin', TMPDIR:'/foreign-temp', DISPLAY: ':0', XAUTHORITY: '/secret', DBUS_SESSION_BUS_ADDRESS: 'foreign', ACCESS_TOKEN: 'private' }, '/tmp/x', 1000);
  assert.equal(env.DISPLAY, undefined);
  assert.equal(env.DBUS_SESSION_BUS_ADDRESS, undefined);
  assert.equal(env.ACCESS_TOKEN, undefined);
  assert.equal(env.XAUTHORITY, '/tmp/x/Xauthority');
  assert.equal(env.TMPDIR,'/tmp/x');
});
test('MP-11 process identity rejects system and invalid signal targets', async () => {
  for (const pid of [0, 1, -1, undefined, NaN, 1.1, Infinity]) assert.equal(validOwnedPid(pid), false);
  assert.equal(await processIdentity(1), null);
  assert.equal(await processIdentity(process.pid), null); // not our child
});

test('MP-08 / MP-11 owned bus names stay bounded and distinct for long state roots', () => {
  const first = desktopBusAddress();
  const second = desktopBusAddress();
  assert.match(first, /^unix:abstract=chariox-desktop-[a-f0-9-]{36}$/);
  assert(first.length < 100);
  assert.notEqual(first, second);
});
test('MP-08 desktop is lazy and failed start cleans its private runtime', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'culinux-unit-'));
  const desktop = new LinuxOwnedDesktop(root, { uid: 1000, commands: { Xvfb: '/does/not/exist' } });
  try {
    assert.equal(desktop.binding(), null);
    await assert.rejects(desktop.start(), /desktop/);
    assert.equal(desktop.binding(), null);
    assert.equal((await stat(root)).isDirectory(), true);
    await desktop.stop();
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('MP-11 stale root identity cannot acquire descendants or signal a replacement',async()=>{
 const {spawn}=await import('node:child_process');
 const child=spawn('/usr/bin/python3',['-c','import time;time.sleep(30)'],{stdio:'ignore'});
 const identity=await processIdentity(child.pid);assert(identity);
 try{
  const stale={...identity,started:'0'};
  assert.deepEqual(await descendants([stale]),[]);
  assert.equal(await signalOwned(stale,'SIGTERM'),false);
  for(const pid of [0,1,-1,undefined,NaN])await assert.rejects(signalOwned({pid},'SIGTERM'),/invalid/);
 }finally{await settleOwned([{child,identity}]);}
});

test('MP-08 / MP-11 Xvfb uses owned private tmp without global socket or keymap writes', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'culinux-private-tmp-'));
  const desktop = new LinuxOwnedDesktop(root);
  desktop.uid = 1000;
  let launched;
  desktop.launch = async (name, args, env) => {
    launched = { name, args, env, runtime: desktop.runtime };
    throw new Error('MP-08: stop before executing Xvfb');
  };
  try {
    await assert.rejects(desktop.start(), /stop before executing/);
    assert.equal(launched.name, 'bwrap');
    assert.deepEqual(launched.args.slice(0, 7), [
      '--dev-bind', '/', '/', '--bind', launched.runtime, '/tmp', '--die-with-parent',
    ]);
    assert.equal(launched.env.TMPDIR, '/tmp');
    assert.equal(launched.env.XAUTHORITY, '/tmp/Xauthority');
    assert(launched.args.includes('-displayfd'));
    assert(launched.args.includes('tcp'));
    assert(!launched.args.includes('-ac'));
    assert(!launched.args.includes('--unshare-net'));
    await assert.rejects(stat(launched.runtime), { code: 'ENOENT' });
  } finally { await desktop.stop(); await rm(root, { recursive: true, force: true }); }
});
