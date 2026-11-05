// MP-08 / MP-10 / MP-11: the terminal watches; the kernel builds and installs.
import assert from "node:assert/strict"
import test from "node:test"
import { mkdtemp, mkdir, writeFile, rm, realpath } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { AppDevLoop, type AppDevDeps } from "./app-dev-loop.js"

test("dev saves use only the kernel push, debounce bursts and recover after errors", async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "app-push-watch-")))
  await mkdir(join(root, "bundle")); await writeFile(join(root, "app.json"), "{}")
  t.after(() => rm(root, { recursive: true, force: true }))
  let change!: (file: string | null) => void
  const notices: string[] = [], begins: any[] = []
  let broken = false
  const loop = new AppDevLoop({
    currentSession: () => "room", notice: text => notices.push(text), debounceMs: 5, pollMs: 1,
    watch: (_root, callback) => { change = callback; return { close() {} } },
    pack: async () => { assert.fail("client must not pack a dev build") },
    installer: new Proxy({}, { get() { assert.fail("client must not run the dev installer") } }) as AppDevDeps["installer"],
    send: async request => {
      if (request.BeginAppDevPush) { begins.push(request.BeginAppDevPush); return { AppDevPushBuilding: { request_id: (request.BeginAppDevPush as any).request_id } } }
      const id = (request.GetAppDevPush as any)?.request_id
      assert.ok(id, "watch adapter must poll the shared kernel push")
      return broken ? { AppDevPushFailed: { request_id: id, message: "syntax error at app.ts:3" } } : {
        AppInstallOperationStatus: { operation: { request_id: id, phase: "committed", installation_id: "dev-todo", generation: String(begins.length), package_digest: "sha256:" + "a".repeat(64), interaction_id: null, failure: null } },
      }
    },
  })
  t.after(() => loop.dispose())
  await loop.start(root); await loop.settled()
  assert.equal(begins.length, 1)
  change("app.ts"); change("bundle/ui/index.html"); change("app.ts")
  await loop.settled(); assert.equal(begins.length, 2)
  broken = true; change("app.ts"); await loop.settled()
  assert.ok(notices.some(text => text.includes("syntax error at app.ts:3")))
  broken = false; change("app.ts"); await loop.settled()
  assert.equal(begins.length, 4)
  assert.ok(notices.some(text => text.includes("generation 4")))
  assert.equal(begins[0].directory, root)
  assert.equal(begins[0].session_id, "room")
})

test("saves during a push coalesce into one newest revision without overlapping requests", async t => {
  const root=await realpath(await mkdtemp(join(tmpdir(),"app-push-order-")))
  await mkdir(join(root,"bundle"));await writeFile(join(root,"app.json"),"{}")
  t.after(()=>rm(root,{recursive:true,force:true}))
  let change!:(file:string|null)=>void, release!:()=>void, entered!:()=>void
  const gate=new Promise<void>(done=>{release=done}), started=new Promise<void>(done=>{entered=done})
  const revisions:number[]=[], ids:string[]=[]
  let revision=0, active=0, maxActive=0, first=true
  const loop=new AppDevLoop({currentSession:()=>"room",notice:()=>{},debounceMs:5,pollMs:1,
    installer:{} as AppDevDeps["installer"],
    watch:(_root,callback)=>{change=callback;return{close(){}}},
    send:async request=>{
      if(request.BeginAppDevPush){
        active++;maxActive=Math.max(maxActive,active);revisions.push(revision)
        const id=(request.BeginAppDevPush as any).request_id;ids.push(id)
        return{AppDevPushBuilding:{request_id:id}}
      }
      if(first){first=false;entered();await gate}
      active--
      const id=(request.GetAppDevPush as any).request_id
      return{AppInstallOperationStatus:{operation:{request_id:id,phase:"committed",installation_id:"dev",generation:String(ids.length),package_digest:"sha256:"+"a".repeat(64),interaction_id:null,failure:null}}}
    },
  })
  t.after(()=>loop.dispose())
  await loop.start(root);await started
  revision=1;change("app.ts");revision=2;change("app.ts");revision=3;change("bundle/ui/index.html")
  await new Promise(done=>setTimeout(done,15))
  assert.equal(ids.length,1)
  release();await loop.settled()
  assert.deepEqual(revisions,[0,3]);assert.equal(maxActive,1)
})

test("a lost begin response is replayed before submitting the next saved revision",async t=>{
  const root=await realpath(await mkdtemp(join(tmpdir(),"app-push-replay-")))
  await mkdir(join(root,"bundle"));await writeFile(join(root,"app.json"),"{}")
  t.after(()=>rm(root,{recursive:true,force:true}))
  let change!:(file:string|null)=>void, lost=true
  const ids:string[]=[], notices:string[]=[]
  const loop=new AppDevLoop({currentSession:()=>"room",notice:text=>notices.push(text),debounceMs:1,pollMs:1,
    installer:{} as AppDevDeps["installer"],watch:(_root,callback)=>{change=callback;return{close(){}}},
    send:async request=>{
      const id=(request.BeginAppDevPush as any)?.request_id
      assert.ok(id)
      ids.push(id)
      if(lost){lost=false;throw new Error("connection lost after admission")}
      return{AppInstallOperationStatus:{operation:{request_id:id,phase:"committed",installation_id:"dev",generation:String(ids.length),package_digest:"sha256:"+"a".repeat(64),interaction_id:null,failure:null}}}
    },
  })
  t.after(()=>loop.dispose())
  await loop.start(root);await loop.settled();change("app.ts");await loop.settled()
  assert.equal(ids.length,3);assert.equal(ids[0],ids[1]);assert.notEqual(ids[1],ids[2])
  assert.ok(notices.some(text=>text.startsWith("Previous build:")))
})

for (const code of ["busy", "storage_unavailable"]) {
 test(`${code} status admission keeps the accepted push identity for the next save`,async t=>{
  const root=await realpath(await mkdtemp(join(tmpdir(),"app-push-busy-")))
  await mkdir(join(root,"bundle"));await writeFile(join(root,"app.json"),"{}")
  t.after(()=>rm(root,{recursive:true,force:true}))
  let change!:(file:string|null)=>void, busy=true
  const ids:string[]=[]
  const committed=(id:string)=>({AppInstallOperationStatus:{operation:{request_id:id,phase:"committed",installation_id:"dev",generation:String(ids.length),package_digest:"sha256:"+"a".repeat(64),interaction_id:null,failure:null}}})
  const loop=new AppDevLoop({currentSession:()=>"room",notice:()=>{},debounceMs:1,pollMs:1,
    installer:{} as AppDevDeps["installer"],watch:(_root,callback)=>{change=callback;return{close(){}}},
    send:async request=>{
      const id=(request.BeginAppDevPush as any)?.request_id
      if(id){ids.push(id);return ids.length===1?{AppDevPushBuilding:{request_id:id}}:committed(id)}
      assert.ok(request.GetAppDevPush)
      assert.equal(busy,true);busy=false;return{AppRequestFailed:{code}}
    },
  })
  t.after(()=>loop.dispose())
  await loop.start(root);await loop.settled();change("app.ts");await loop.settled()
  assert.equal(ids.length,3);assert.equal(ids[0],ids[1]);assert.notEqual(ids[1],ids[2])
})
}

test("a failed build found after a lost response still pushes the newest save", async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "app-push-failed-replay-")))
  await mkdir(join(root, "bundle")); await writeFile(join(root, "app.json"), "{}")
  t.after(() => rm(root, { recursive: true, force: true }))
  let change!: (file: string | null) => void
  const ids: string[] = [], notices: string[] = []
  const loop = new AppDevLoop({
    currentSession: () => "room", notice: text => notices.push(text), debounceMs: 1, pollMs: 1,
    installer: {} as AppDevDeps["installer"],
    watch: (_root, callback) => { change = callback; return { close() {} } },
    send: async request => {
      const id = (request.BeginAppDevPush as any)?.request_id
      assert.ok(id); ids.push(id)
      if (ids.length === 1) throw new Error("connection lost after admission")
      if (ids.length === 2) return { AppDevPushFailed: { request_id: id, code: "build_failed", message: "Earlier source had a syntax error" } }
      return { AppInstallOperationStatus: { operation: { request_id: id, phase: "committed", installation_id: "dev", generation: "2", package_digest: "sha256:" + "a".repeat(64), interaction_id: null, failure: null } } }
    },
  })
  t.after(() => loop.dispose())
  await loop.start(root); await loop.settled(); change("app.ts"); await loop.settled()
  assert.equal(ids.length, 3)
  assert.equal(ids[0], ids[1]); assert.notEqual(ids[1], ids[2])
  assert.ok(notices.some(text => text.startsWith("Previous build:") && text.includes("syntax error")))
  assert.ok(notices.some(text => text.includes("generation 2")))
})

// MP-08 / MP-10 / MP-11: a rejected begin never owns a source revision.
for (const code of ["busy", "storage_unavailable"]) {
  test(`a begin rejected as ${code} is not replayed on the next save`, async t => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "app-push-rejected-")))
    await mkdir(join(root, "bundle")); await writeFile(join(root, "app.json"), "{}")
    t.after(() => rm(root, { recursive: true, force: true }))
    let change!: (file: string | null) => void
    const ids: string[] = [], notices: string[] = []
    const loop = new AppDevLoop({ currentSession: () => "room", notice: text => notices.push(text), debounceMs: 1, pollMs: 1,
      installer: {} as AppDevDeps["installer"], watch: (_root, callback) => { change = callback; return { close() {} } },
      send: async request => {
        const id = (request.BeginAppDevPush as any)?.request_id
        assert.ok(id); ids.push(id)
        if (ids.length === 1) return { AppRequestFailed: { code } }
        return { AppInstallOperationStatus: { operation: { request_id: id, phase: "committed", installation_id: "dev", generation: "1", package_digest: "sha256:" + "a".repeat(64), interaction_id: null, failure: null } } }
      },
    })
    t.after(() => loop.dispose())
    await loop.start(root); await loop.settled()
    change("app.ts"); await loop.settled()
    assert.equal(ids.length, 2, "one accepted begin for one save")
    assert.notEqual(ids[0], ids[1])
    assert.ok(notices.some(text => text.includes(code) && text.includes("save again to retry")))
  })
}
