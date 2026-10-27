#!/usr/bin/env node
// Run with slot-run. Uses only a disposable kernel, public signed fixture,
// private Docker slice and temporary HOME; never reads provider credentials.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile, rm, chmod } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const args = Object.fromEntries(Array.from({length: process.argv.slice(2).length / 2}, (_, i) => process.argv.slice(2 + i * 2, 4 + i * 2)));
for (const key of ["--kernel", "--image", "--source-ref", "--app-package", "--publisher-public", "--evidence", "--expect"]) assert.ok(args[key], `${key} is required`);
assert.ok(["broken", "recovered"].includes(args["--expect"]));
const evidence = path.resolve(args["--evidence"]);
await mkdir(evidence, {recursive:true});
const scratch = await mkdtemp(path.join(os.tmpdir(), "room-browser-recovery-"));
const run = `browser-recovery-${process.pid}`;
const container = `chariox-slice-${run}`;
const home = path.join(scratch,"home");
const workspace = path.join(scratch,"workspace");
const resources = path.join(scratch,"resources");
const receipts = [];
const docker = (...argv) => execFileSync("docker", argv, {encoding:"utf8",timeout:120000}).trim();
const delay = ms => new Promise(resolve => setTimeout(resolve,ms));
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
let kernel, session, slice;
let failure;
const port = Number(args["--port"] ?? 57410);
async function rpc(request, optional = false) {
  const result = await new Promise((resolve,reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/kernel`);
    const id = randomUUID();
    let settled = false;
    const timer = setTimeout(()=>{settled=true;ws.close();reject(new Error("kernel request timeout"));},180000);
    ws.onopen=()=>ws.send(JSON.stringify({type:"request",request_id:id,command_id:id,request}));
    ws.onmessage=({data})=>{
      const frame=JSON.parse(data);
      if(frame.request_id!==id) return;
      settled=true;clearTimeout(timer);ws.close();resolve(frame);
    };
    ws.onerror=()=>{if(!settled){clearTimeout(timer);reject(new Error("kernel connection failed"));}};
  });
  receipts.push({at:new Date().toISOString(),request,result});
  if (!optional) assert.equal(result.error,null,JSON.stringify(result.error));
  return result.response;
}
async function waitFor(check, timeout = 30000) {
  const deadline = Date.now()+timeout;
  let last;
  while(Date.now()<deadline){try{last=await check();if(last)return last;}catch(error){if(error instanceof assert.AssertionError)throw error;last=error;}await delay(250);}
  throw new Error(`condition timed out: ${last}`);
}
async function state() { return (await rpc({GetRoomEnvironmentState:{session_id:session}})).RoomEnvironmentState.environment; }
function browserHealth(environment) { return environment.health.find(h=>h.component==="browser"); }
function browserPid() {
  return Number(docker("exec",container,"node","-e",`const fs=require('fs');for(const p of fs.readdirSync('/proc').filter(p=>/^\\d+$/.test(p))){try{const a=fs.readFileSync('/proc/'+p+'/cmdline','utf8').split('\\0');if(a[0]?.endsWith('/chromium')&&a.some(x=>x.startsWith('--user-data-dir='))&&!a.some(x=>x.startsWith('--type='))){console.log(p);break;}}catch{}}`));
}
async function installFixture() {
  const publisher=JSON.parse(await readFile(args["--publisher-public"],"utf8"));
  const enrollment=randomUUID();
  let response=await rpc({BeginAppPublisherEnrollment:{session_id:session,request_id:enrollment,publisher_id:publisher.publisher.id,key_id:publisher.publisher.keyId,public_key_base64:publisher.publicKey,expected_revision:"0"}});
  await waitFor(async()=>{
    const operation=response.AppPublisherEnrollmentStatus.operation;
    if(operation.interaction_id) await rpc({RespondToInteraction:{session_id:session,interaction_id:operation.interaction_id,choice_id:"approve"}},true);
    if(operation.phase==="approved")return true;
    assert.notEqual(operation.phase,"failed");
    response=await rpc({GetAppPublisherEnrollment:{request_id:enrollment}});return false;
  });
  const bytes=await readFile(args["--app-package"]);
  const request_id=randomUUID();
  const upload=(await rpc({BeginAppPackageUpload:{request_id,expected_size:bytes.length,sha256:`sha256:${hash(bytes)}`}})).AppPackageUploadStatus.upload;
  for(let offset=0;offset<bytes.length;offset+=196608){const chunk=bytes.subarray(offset,offset+196608);await rpc({PutAppPackageUploadChunk:{handle:upload.handle,offset,data_base64:chunk.toString("base64"),chunk_sha256:`sha256:${hash(chunk)}`}});}
  response=await rpc({BeginAppInstall:{session_id:session,request_id,upload_handle:upload.handle,expected_package_digest:`sha256:${hash(bytes)}`}});
  return await waitFor(async()=>{
    const operation=response.AppInstallOperationStatus.operation;
    if(operation.interaction_id)await rpc({RespondToInteraction:{session_id:session,interaction_id:operation.interaction_id,choice_id:"approve"}},true);
    if(operation.phase==="committed")return operation.installation_id;
    if(operation.phase==="failed" && operation.failure==="app_lifecycle_preparation" && args["--view-only-fixture"]==="yes") {
      // This builder has no enrolled native App runtime. Seed only this
      // disposable database's approved, verifier-produced UI catalog. The
      // normal OpenAppView path still verifies signature and publisher trust;
      // this is not evidence for native worker installation or execution.
      const seed = `import sqlite3,json,sys
c=sqlite3.connect(sys.argv[1]);i=sys.argv[2]
r=json.loads(c.execute('SELECT record_json FROM app_installation_updates WHERE installation_id=? ORDER BY generation DESC LIMIT 1',(i,)).fetchone()[0])
assert r['decision']['status']=='approved'
a={'generation':r['token']['generation'],'release':r['release'],'approval':r['decision']['approval']}
c.execute('UPDATE app_installations SET generation=?,active_json=?,pending_generation=NULL,admission_paused=0 WHERE installation_id=?',(a['generation'],json.dumps(a),i))
c.commit()
`;
      execFileSync("python3",["-c",seed,path.join(scratch,"state.db"),operation.installation_id]);
      receipts.push({viewOnlyCatalogFixture:operation.installation_id,reason:"no system-enrolled native runtime; only browser UI recovery is in scope"});
      return operation.installation_id;
    }
    assert.ok(!["failed","cancelled"].includes(operation.phase),JSON.stringify(operation));
    response=await rpc({GetAppInstallOperation:{request_id}});return false;
  },120000);
}
async function closeTab(tab_id) {
  await rpc({RequestRoomEnvironmentInputTakeover:{session_id:session,target:{kind:"browser_tab",id:tab_id}}});
  const environment=await state();
  return await rpc({SubmitRoomEnvironmentBrowserAction:{session_id:session,runtime_generation:environment.runtime_generation,idempotency_key:randomUUID(),action:{kind:"tab",tab_id,action:"close"}}},true);
}
async function open(installation_id, optional=false) {return await rpc({OpenAppView:{session_id:session,installation_id}},optional);}

try {
  await Promise.all([home,workspace,resources].map(p=>mkdir(p,{recursive:true})));
  await chmod(workspace,0o755);
  // Archive only public provisioner resources from the requested input; this
  // is not another worktree. Pin the existing image's attested resource hash
  // so this drill never rebuilds or retags a shared image.
  const archive=execFileSync("git",["archive",args["--source-ref"],"apps/kernel/slice-linux-docker","apps/kernel/src/transport/relay_peer.rs","apps/browser-session-import"],{cwd:repo,maxBuffer:16*1024*1024});
  execFileSync("tar",["-x","-C",resources],{input:archive});
  const provisioner=path.join(resources,"apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh");
  const imageRevision=docker("image","inspect","-f",'{{ index .Config.Labels "io.chariox.runtime-source-revision" }}',args["--image"]);
  assert.match(imageRevision,/^(sha256:)?[a-f0-9]{64}$/);
  let provision=await readFile(provisioner,"utf8");
  provision=provision.replace("runtime_source_revision() {",`runtime_source_revision() {\n  printf '%s\\n' '${imageRevision}'; return\n`);
  await writeFile(provisioner,provision);
  // The after image carries candidate-k resources plus the actual PR patch.
  // Overlay the same patched public resource files for the provisioner.
  if(args["--patch"]){execFileSync("patch",["-p1","--batch","-i",path.resolve(args["--patch"])],{cwd:resources});}
  await writeFile(path.join(home,"config.toml"),`version = 1\n[credential_vault]\nbackend = "process_memory"\npath = "${scratch}/vault.json"\nservice = "${run}"\nagent_management = "allow"\n[state]\npath = "${scratch}/state.db"\n[slices]\nroot = "${scratch}/slices"\n[slices.linux]\ndocker_image = "${args["--image"]}"\nbuild_image = "never"\nmemory_mb = 2048\ncpus = "2.0"\n`);
  // A small explicit environment prevents accidental inheritance of live
  // kernel/provider configuration. No auth files, keys or accounts are copied.
  const env={PATH:process.env.PATH,HOME:scratch,LANG:"C.UTF-8",CHARIOX_HOME:home,XDG_CONFIG_HOME:path.join(scratch,"config"),CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT:"1",CHARIOX_KERNEL_PORT:String(port),CHARIOX_MCP_PORT:String(port+1),CHARIOX_CODEX_PORT:String(port+2),CHARIOX_OPENCODE_PORT:String(port+3),CHARIOX_DAEMON_ID:run,CHARIOX_SLICE_DOCKER_PROVISIONER:provisioner};
  kernel=spawn(path.resolve(args["--kernel"]),[],{cwd:repo,env,stdio:["ignore","pipe","pipe"]});
  let output="";kernel.stdout.on("data",b=>output+=b);kernel.stderr.on("data",b=>output+=b);
  await waitFor(()=>rpc({ListSlices:{}},true),30000);
  session=(await rpc({CreateSession:{workspace_id:workspace,worktree_id:workspace,agent_defaults:{provider:"dev-stub"}}})).SessionCreated.session.id;
  slice=(await rpc({CreateSlice:{name:run,backend:"local_docker",display_mode:"headed",display_backend:"novnc",workspace_mount:workspace}})).SliceCreated.slice.id;
  await rpc({BindRoomEnvironmentSlice:{session_id:session,slice_ref:slice}});
  await rpc({StartSlice:{slice_ref:slice}});
  const startedAt=docker("inspect","-f","{{.State.StartedAt}}",container);
  await rpc({StartRoomEnvironment:{session_id:session,viewport:{css_width:1280,css_height:800,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:800}}});
  assert.equal((await state()).lifecycle,"ready");
  const installation=await installFixture();
  const opened=await open(installation);
  const initial=await state();
  const app=initial.tabs.find(tab=>tab.url.startsWith(opened.AppViewOpened.origin));
  assert.ok(app,"App view must be projected as a Room tab");
  for(const tab of initial.tabs.filter(tab=>tab.tab_id!==app.tab_id))await closeTab(tab.tab_id);
  const closePid=browserPid();assert.ok(closePid);
  await closeTab(app.tab_id);
  await delay(1000);
  const closeState=await state();
  const closeOpen=await open(installation,true);
  const lastTab={pidBefore:closePid,pidAfter:browserPid(),environment:closeState,openResult:closeOpen};
  if(args["--expect"]==="broken"){
    assert.equal(lastTab.pidAfter,0);assert.equal(browserHealth(closeState).state,"ready");assert.ok(!closeOpen?.AppViewOpened);
    // Repair just the browser manually to exercise the second pre-fix fault.
    docker("exec",container,"/opt/chariox-slice/slice-screen.sh","open-url","about:blank");
    await rpc({StopRoomEnvironment:{session_id:session}});
    await rpc({StartRoomEnvironment:{session_id:session,viewport:initial.viewport}});
  }else{
    assert.equal(lastTab.pidAfter,closePid);assert.equal(browserHealth(closeState).state,"ready");assert.ok(closeOpen?.AppViewOpened);
  }
  await open(installation);
  const termPid=browserPid();assert.ok(termPid);
  docker("exec",container,"kill","-TERM",String(termPid));
  const killedAt=Date.now();
  let unavailable;
  if(args["--expect"]==="recovered"){
    unavailable=await waitFor(async()=>{const s=await state();return browserHealth(s).state==="unavailable"&&s;},10000);
    assert.ok(browserHealth(unavailable).diagnostic_code);
    const detectedMs=Date.now()-killedAt;
    const recovered=await waitFor(async()=>{const s=await state();return s.lifecycle==="ready"&&browserPid()>0&&s;},30000);
    const reopened=await open(installation);
    assert.ok(reopened.AppViewOpened);
    receipts.push({term:{pid:termPid,detectedMs,recoveredMs:Date.now()-killedAt,unavailable,recovered,reopened}});
  }else{
    await delay(11000);
    const stale=await state();assert.equal(browserHealth(stale).state,"ready");assert.equal(browserPid(),0);
    assert.ok(!(await open(installation,true))?.AppViewOpened);
    await rpc({StopRoomEnvironment:{session_id:session}});
    await rpc({StartRoomEnvironment:{session_id:session,viewport:initial.viewport}},true);
    await rpc({RetryRoomEnvironment:{session_id:session}},true);
    assert.equal(browserHealth(await state()).state,"unavailable");
    receipts.push({term:{pid:termPid,stale}});
  }
  assert.equal(docker("inspect","-f","{{.State.StartedAt}}",container),startedAt,"recovery must not restart the slice");
  receipts.push({lastTab,image:args["--image"],imageId:docker("image","inspect","-f","{{.Id}}",args["--image"]),kernelSha256:hash(await readFile(args["--kernel"])),processes:docker("exec",container,"ps","-eo","pid,ppid,stat,comm")});
  await writeFile(path.join(evidence,"kernel-stdout.log"),output);
}catch(error){failure=error;}
finally{
  if(session)await rpc({EndSession:{session_id:session}},true).catch(()=>{});
  if(slice){await rpc({StopSlice:{slice_ref:slice}},true).catch(()=>{});await rpc({DeleteSlice:{slice_ref:slice}},true).catch(()=>{});}
  if(kernel){kernel.kill("SIGTERM");await Promise.race([new Promise(resolve=>kernel.once("exit",resolve)),delay(3000)]);if(kernel.exitCode===null)kernel.kill("SIGKILL");}
  // Only the drill's exact container/volume names are cleanup targets.
  try{docker("rm","-f",container);}catch{}
  try{docker("volume","rm",`${container}-home`);}catch{}
  await writeFile(path.join(evidence,"receipts.json"),JSON.stringify({passed:!failure,expected:args["--expect"],failure:failure?.stack,receipts},null,2));
  await rm(scratch,{recursive:true,force:true});
}
if(failure)throw failure;
console.log(`ROOM_BROWSER_RECOVERY_PASS ${args["--expect"]} ${evidence}`);
