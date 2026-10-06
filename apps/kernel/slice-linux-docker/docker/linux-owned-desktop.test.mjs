// MP-08 / MP-11: fail-first host lifecycle contracts.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { LinuxOwnedDesktop, desktopEnvironment } from './linux-owned-desktop.mjs';
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
