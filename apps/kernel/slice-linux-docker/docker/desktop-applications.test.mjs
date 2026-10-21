// MP-08/MP-11: real installed-descriptor parsing; supplementary to Drill D.
import test from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtemp, mkdir, writeFile, rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {fileURLToPath} from 'node:url';
import {desktopApplication} from './desktop-applications.mjs';
const helper=fileURLToPath(new URL('./desktop-applications.py',import.meta.url));
const python=process.env.CHARIOX_DESKTOP_TEST_PYTHON??'/usr/bin/python3';
const environment={...process.env};
test('MP-11 installed IDs never admit paths, commands, arguments or environment',async()=>{
 for(const request of [
  {op:'resolve',application_id:'../probe.desktop'},
  {op:'resolve',application_id:'probe.desktop',args:['secret']},
  {op:'resolve',application_id:'probe.desktop',env:{DISPLAY:':0'}},
  {op:'resolve',application_id:'sh -c probe'},
  {op:'resolve',application_id:'does-not-exist.desktop'},
 ])await assert.rejects(desktopApplication(request,environment),/installed application refused/);
});
test('MP-08 installed public entry resolves fixed argv and omits callerless file field codes',async()=>{
 const root=await mkdtemp(path.join(os.tmpdir(),'desktop-app-test-'));
 const applications=path.join(root,'applications');await mkdir(applications);
 const env={...environment,XDG_DATA_HOME:root,XDG_DATA_DIRS:root};
 const read=request=>JSON.parse(execFileSync(python,[helper],{env,input:JSON.stringify(request),stdio:['pipe','pipe','pipe']}).toString());
 try {
  await writeFile(path.join(applications,'probe.desktop'),'[Desktop Entry]\nType=Application\nName=Public editor\nExec=/usr/bin/true "ordinary text" %% %c %k %U\nTerminal=false\n');
  const listed=read({op:'list'});assert.deepEqual(listed,{applications:[{application_id:'probe.desktop',name:'Public editor'}],truncated:false});
  const resolved=read({op:'resolve',application_id:'probe.desktop'});
  assert.deepEqual(resolved.argv,['/usr/bin/true','ordinary text','%','Public editor',path.join(applications,'probe.desktop')]);
  for(const exec of ['/bin/sh -c "echo forbidden"','/usr/bin/env DISPLAY=:0 true','/usr/bin/true %Z']) {
   await writeFile(path.join(applications,'probe.desktop'),'[Desktop Entry]\nType=Application\nName=Public editor\nExec='+exec+'\nTerminal=false\n');
   assert.throws(()=>read({op:'resolve',application_id:'probe.desktop'}));
  }
  await writeFile(path.join(applications,'probe.desktop'),'[Desktop Entry]\nType=Application\nName=Public editor\nExec=/usr/bin/true\nTerminal=true\n');
  assert.throws(()=>read({op:'resolve',application_id:'probe.desktop'}));
 }finally{await rm(root,{recursive:true,force:true});}
});
