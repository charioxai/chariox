// MP-08/MP-10/MP-11: headed ownership without video, supplementary native seam.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import path from 'node:path';
import {HostChromium} from './kernel-browser-process.mjs';
import {ownsDisplay} from './kernel-browser-owned-display.mjs';

test('MP-08/MP-10/MP-11 headed mirror owns its display with video disabled', async()=>{
  assert(process.getuid() > 0, 'normal Unix user required');
  assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'explicit external drill root required');
  const root=await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT,'mirror-headed-'));
  const environment={...process.env,CHARIOX_KERNEL_BROWSER_MIRROR:'1',CHARIOX_KERNEL_BROWSER_DISPLAY:'0',CHARIOX_KERNEL_BROWSER_HEADLESS:'0'};
  delete environment.DISPLAY;delete environment.XAUTHORITY;
  const chromium=new HostChromium(root,{environment});
  try {
    const connection=await chromium.start();
    assert(ownsDisplay(chromium.display), 'mirror owns a private headed display independently of video');
    assert.match((await connection.send('Browser.getVersion')).product,/Chrome|Chromium/);
    assert.equal(chromium.child.exitCode,null);
  } finally {await chromium.stop();await rm(root,{recursive:true,force:true});}
  assert.equal(chromium.child,null);assert.equal(chromium.display,null);
});
