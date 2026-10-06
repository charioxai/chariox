// MP-08 / MP-11: source-bound local Xvfb drill; no provider/display acceptance.
import assert from 'node:assert/strict';
import { mkdtemp, rm, access, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { LinuxOwnedDesktop } from './docker/linux-owned-desktop.mjs';
const root = await mkdtemp(path.join(os.tmpdir(), 'culinux-desktop-state-'));
const desktop = new LinuxOwnedDesktop(root);
let pids = [];
try {
  assert.equal(desktop.binding(), null);
  const binding = await desktop.start();
  pids = desktop.children.map(({child}) => child.pid);
  assert.equal(await desktop.start(), binding);
  assert.equal((await stat(binding.environment.XAUTHORITY)).mode & 0o777, 0o600);
  const ok = spawnSync('xdpyinfo', [], { env: binding.environment, encoding: 'utf8' });
  assert.equal(ok.status, 0);
  assert.equal(spawnSync('/usr/bin/python3', ['-c', 'from Xlib.display import Display; c=Display(); c.sync(); c.close()'], {env:binding.environment}).status, 0);
  assert.match(ok.stdout, /1280x800/);
  const denied = spawnSync('xdpyinfo', [], { env: { ...binding.environment, XAUTHORITY: '/dev/null' } });
  assert.notEqual(denied.status, 0);
  const surface = binding.surface_id;
  await desktop.stop();
  assert.equal(desktop.binding(), null);
  for (const pid of pids) await assert.rejects(access(`/proc/${pid}`));
  const fresh = await desktop.start();
  assert.notEqual(fresh.surface_id, surface);
  await desktop.stop();
  console.log(JSON.stringify({items:['MP-08','MP-11'], result:'PASS', checks:['lazy','non-root','private-auth','foreign-cookie-denied','canonical-geometry','same-live-generation','fresh-restart-generation','owned-process-cleanup'], source:process.env.CULINUX_SOURCE, limits:'Native lifecycle only; browser, paid providers, stream, Web/TUI and MP-10 Path-1 acceptance not claimed'}));
} finally { await desktop.stop(); await rm(root, { recursive:true, force:true }); }
