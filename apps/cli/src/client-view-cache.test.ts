// MP-08 / MP-10 / MP-11: private presentation never crosses an identity or kernel.
import assert from "node:assert/strict"
import test from "node:test"
import {mkdtemp,rm,stat,chmod} from "node:fs/promises"
import {join} from "node:path"
import {tmpdir} from "node:os"
import {readClientView,writeClientView,clearClientViews,clientViewEpoch,type CliCachedView} from "./client-view-cache.js"
import {preferencesPath} from "./preferences.js"
import {dirname} from "node:path"

test("MP-08 / MP-10 / MP-11 TUI view files are private, bounded, scoped and invalidated",async()=>{
 const root=await mkdtemp(join(tmpdir(),"panes-view-cache-")),previous=process.env.CHARIOX_HOME
 process.env.CHARIOX_HOME=root
 const scope={owner:'["cloud","user-a","account-a","realm-a"]',kernel:"kernel-a"}
 const view=(agentId="agent-a",savedAt=Date.now()):CliCachedView=>({scope,sessionId:"session-a",agentId,alias:"source review",sequence:30,savedAt,entries:[{id:1,role:"assistant",text:"literal <chariox-context>\nsource review",historyEntryIndex:30}]})
 try{
  await clearClientViews();await writeClientView(view())
  assert.equal((await readClientView(scope,"session-a"))?.sequence,30)
  assert.equal(await readClientView({...scope,owner:"user-b"},"session-a"),null)
  assert.equal(await readClientView({...scope,kernel:"kernel-b"},"session-a"),null)
  assert.equal(await readClientView(scope,"session-b"),null)
  assert.equal(await readClientView(scope,"session-a","agent-b"),null)
  const file=join(dirname(preferencesPath()),"client-cache","views.json")
  assert.equal((await stat(file)).mode&0o777,0o600)
  await writeClientView(view("agent-a",1));assert.ok((await readClientView(scope,"session-a"))!.savedAt>1)
  const oversized=view();oversized.entries[0]!.text="x".repeat(2*1024*1024);await writeClientView(oversized)
  assert.ok((await readClientView(scope,"session-a"))!.entries[0]!.text.length<100)
  for(let i=0;i<35;i++)await writeClientView(view("agent-"+i,Date.now()+i))
  assert.equal(await readClientView(scope,"session-a","agent-0"),null)
  assert.ok(await readClientView(scope,"session-a","agent-34"))
  await chmod(file,0o644);assert.equal(await readClientView(scope,"session-a"),null);await chmod(file,0o600)
  const generation=clientViewEpoch(),pending=writeClientView(view("late"));await clearClientViews();await pending
  assert.ok(clientViewEpoch()>generation);assert.equal(await readClientView(scope,"session-a"),null)
 }finally{await clearClientViews();if(previous===undefined)delete process.env.CHARIOX_HOME;else process.env.CHARIOX_HOME=previous;await rm(root,{recursive:true,force:true})}
})
