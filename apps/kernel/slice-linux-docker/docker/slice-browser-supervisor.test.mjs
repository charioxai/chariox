import assert from "node:assert/strict";
import test from "node:test";
import { mkdtemp, mkdir, readFile, writeFile, copyFile, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import { once } from "node:events";
import os from "node:os";
import path from "node:path";

const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function waitFor(check, timeout = 12000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { if (await check()) return; await delay(50); }
  assert.fail("supervisor did not reach the expected state");
}

test("browser supervisor owns one child, reaps TERM exits and restarts without losing the profile", {timeout:25000}, async () => {
  const root = await mkdtemp(path.join(os.tmpdir(),"slice-browser-supervisor-"));
  await mkdir(path.join(root,"bin"));
  await copyFile(new URL("./slice-screen.sh",import.meta.url),path.join(root,"slice-screen.sh"));
  await writeFile(path.join(root,"bin/chromium"),`#!/usr/bin/env bash
set -eu
printf '%s\\n' "$$" >> "$CHARIOX_SLICE_ROOT/children"
printf '%s\\n' "$*" >> "$CHARIOX_SLICE_ROOT/arguments"
trap 'exit 0' TERM INT
while true; do sleep 0.1; done
`,{mode:0o755});
  const env={...process.env,PATH:`${root}/bin:${process.env.PATH}`,CHARIOX_SLICE_ROOT:root,CHARIOX_SLICE_CHROME_PROFILE:`${root}/profile`,HOME:root};
  const supervisor=spawn("bash",[path.join(root,"slice-screen.sh"),"supervise-browser"],{env,stdio:"ignore"});
  const exited=once(supervisor,"exit");
  let child;
  try {
    await waitFor(async()=>{try{child=Number((await readFile(path.join(root,"children"),"utf8")).trim());return child>0;}catch{return false;}});
    const duplicate=spawn("bash",[path.join(root,"slice-screen.sh"),"supervise-browser"],{env,stdio:"ignore"});
    assert.equal((await once(duplicate,"exit"))[0],0);
    process.kill(child,"SIGTERM");
    const old=child;
    await waitFor(async()=>{const children=(await readFile(path.join(root,"children"),"utf8")).trim().split("\n");if(children.length!==2)return false;child=Number(children[1]);return true;});
    assert.throws(()=>process.kill(old,0),{code:"ESRCH"},"the previous browser must be reaped");
    assert.ok((await readFile(path.join(root,"arguments"),"utf8")).split("\n").filter(Boolean).every(line=>line.includes(`--user-data-dir=${root}/profile`)));
    supervisor.kill("SIGTERM");
    assert.equal((await exited)[0],0);
    assert.throws(()=>process.kill(child,0),{code:"ESRCH"},"supervisor shutdown must reap its browser");
  }finally{
    if(supervisor.exitCode===null){supervisor.kill("SIGKILL");await exited;}
    if(child){try{process.kill(child,"SIGKILL");}catch{}}
    await rm(root,{recursive:true,force:true});
  }
});
