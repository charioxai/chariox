import assert from "node:assert/strict"
import { test } from "node:test"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { dockerControlPolicy, runBrokerCommand } from "../apps/kernel/slice-linux-docker/managed-broker-command.mjs"

test("MP-08 MP-10 MP-11 raw controls are bounded while archive producers retain owned lifetime", () => {
 for (const command of ["info", "ps", "inspect", "logs", "exec", "image", "container", "start", "stop", "pause", "unpause", "rm", "create"]) {
  assert.equal(dockerControlPolicy([command]).timeout, 30_000, command)
 }
 for (const command of ["commit", "cp"]) assert.equal(dockerControlPolicy([command]).timeout, undefined, command)
})

for (const progress of [false,true]) test(`MP-08 MP-10 MP-11 owned broker command permits ${progress ? "healthy" : "silent"} producers`, async context => {
 const logRoot=await mkdtemp(join(tmpdir(),"chariox-broker-lifetime-"))
 context.after(()=>rm(logRoot,{recursive:true,force:true}))
 const result=await runBrokerCommand("/usr/bin/python3",["-c",progress ? "import time;[(print('progress',flush=True),time.sleep(.1)) for _ in range(4)]" : "import time;time.sleep(.4);print('finished')"],{env:process.env,maxBuffer:4096,logRoot})
 assert.equal(result.status,0,result.stderr.toString())
 assert.match(result.stdout.toString(),progress ? /progress/ : /finished/)
})

test("MP-08 MP-10 MP-11 lease cancellation settles producer and pipe-holding descendant", async context => {
 const root=await mkdtemp(join(tmpdir(),"chariox-broker-owned-"))
 context.after(()=>rm(root,{recursive:true,force:true}))
 const marker=join(root,"pids"), controller=new AbortController()
 const result=runBrokerCommand("/usr/bin/python3",["-c",`import os,signal,time,pathlib
signal.signal(signal.SIGTERM,signal.SIG_IGN)
pid=os.fork()
if pid==0: time.sleep(1000)
else:
 pathlib.Path(${JSON.stringify(marker)}).write_text(str(os.getpid())+' '+str(pid))
 time.sleep(1000)`],{env:process.env,maxBuffer:4096,logRoot:root,signal:controller.signal})
 let pids
 try {
  const until=Date.now()+3000
  while(Date.now()<until){pids=await readFile(marker,"utf8").catch(()=>undefined);if(pids)break;await new Promise(r=>setTimeout(r,20))}
  assert(pids,"producer must start")
 } finally {controller.abort()}
 const settled=await result
 assert.notEqual(settled.status,0)
 for(const pid of pids.split(" ")) {
  const state=await readFile(`/proc/${pid}/stat`,"utf8").catch(()=>undefined)
  assert(!state || state.split(") ")[1].startsWith("Z "),`owned producer ${pid} survived`)
 }
})

test("MP-08 MP-11 verbose successful producer keeps complete private logs and bounded summaries", async context => {
 const root=await mkdtemp(join(tmpdir(),"chariox-broker-output-"))
 context.after(()=>rm(root,{recursive:true,force:true}))
 const count=5*1024*1024+137
 const result=await runBrokerCommand("/usr/bin/python3",["-c",`import os;os.write(1,b'x'*${count});os.write(2,b'y'*${count})`],{env:process.env,maxBuffer:65536,logRoot:root})
 assert.equal(result.status,0,result.stderr.toString())
 assert(result.stdout.length<=65536)
 assert(result.stderr.length<70000)
 const {readdir,stat}=await import("node:fs/promises")
 const [directory]=await readdir(root)
 const logs=join(root,directory)
 for(const [name,byte] of [["stdout.log",120],["stderr.log",121]]) {
  const content=await readFile(join(logs,name))
  assert.equal(content.length,count)
  assert(content.every(value=>value===byte))
  assert.equal((await stat(join(logs,name))).mode&0o777,0o600)
 }
 assert.equal((await stat(logs)).mode&0o777,0o700)
 assert.match(result.stderr.toString(),/complete diagnostics/)
})
