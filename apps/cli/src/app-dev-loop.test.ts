import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdir, mkdtemp, realpath, rm, stat, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, relative } from "node:path"
import test from "node:test"
import { promisify } from "node:util"
import { AppDevLoop, defaultAppDevKey, ignored, type AppDevDeps, type AppDevPackOptions } from "./app-dev-loop.js"

async function fixture(t: test.TestContext, keyParts = ["keys", "app-publisher", "private"]) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "chariox-app-dev-test-")))
  t.after(() => rm(root, { recursive: true, force: true }))
  const app = join(root, "my-app")
  await mkdir(join(app, "bundle"), { recursive: true })
  await writeFile(join(app, "app.json"), "{}")
  const home = join(root, "home")
  const key = join(home, ".chariox", ...keyParts)
  await mkdir(join(key, ".."), { recursive: true, mode: 0o700 })
  // Invalid marker only: the mocked packer never reads or creates a signing key.
  await writeFile(key, "Not a signing key; the packer is mocked.", { mode: 0o600 })
  return { root, app, home, key }
}

function harness(f: { root: string, app: string, home: string, key: string }) {
  const packs: AppDevPackOptions[] = [], workspaces: string[] = []
  const calls: {kind: string, installation?: string, source?: string | undefined}[] = []
  const status = { request_id:"register", phase:"committed", installation_id:"registered", generation:"2", package_digest:"sha256:"+"a".repeat(64), interaction_id:null, failure:null } as const
  const deps: AppDevDeps = {
    cwd:f.root, home:f.home, currentSession:()=>"session-1", notice:()=>{},
    workspace:async()=>{const dir=await mkdtemp(join(f.root,"work-"));workspaces.push(dir);return dir},
    pack:async options=>{packs.push(options);return {appId:"com.example.todo",version:"1.0.0",packageDigest:status.package_digest}},
    send:async()=>({AppInstallationsListed:{installations:[],next_cursor:null}}),
    installer:{
      install:async(_path,_session,source)=>{calls.push({kind:"install",source});return status},
      update:async(installation,_path,_session,source)=>{calls.push({kind:"update",installation,source});return status},
      status:async()=>status,cancel:async()=>undefined,retained:()=>undefined,discardRetained:()=>true,
    },
  }
  return {deps,packs,workspaces,calls}
}

test("missing-key guidance uses protected paths and private directory modes without creating a key", async t => {
  const f = await fixture(t)
  const home = join(f.root, "empty-home")
  await mkdir(home)
  const h = harness(f)
  const loop = new AppDevLoop({ ...h.deps, home })
  let instructions = ""
  await assert.rejects(loop.register("my-app"), (error: Error) => {
    instructions = error.message
    assert.ok(instructions.startsWith(`No developer signing key at ${defaultAppDevKey(home)}.\n`))
    return true
  })
  assert.ok(instructions.includes('--key-out "$HOME/.chariox/keys/app-publisher/private"'))
  assert.ok(instructions.includes('--trust-out "$HOME/.chariox/keys/app-publisher/publisher.json"'))
  assert.ok(instructions.includes('/app publisher enroll "$HOME/.chariox/keys/app-publisher/publisher.json"'))
  assert.match(instructions, /--key PRIVATE for an existing key at its original location/)
  assert.match(instructions, /do not generate a replacement or move it/)
  assert.match(instructions, /directory 0700, private file 0600/)
  assert.match(instructions, /same public fingerprint, and preserved backup history/)
  assert.match(instructions, /separately protected backup.*signing and verifying a synthetic challenge/)
  // MP-11: the first line names the requested HOME, which a disk-backed test
  // TMPDIR may place under dev. The storage advice must never recommend dev.
  const storageAdvice = instructions.slice(instructions.indexOf("\n") + 1)
  assert.doesNotMatch(storageAdvice, /\.chariox\/dev\//)
  assert.equal(h.packs.length, 0)
  assert.equal(h.workspaces.length, 0)
  await assert.rejects(stat(join(home, ".chariox")), /ENOENT/)
  // Execute only the directory setup line in an empty synthetic HOME, never keygen.
  const setup = instructions.split("\n").find(line => line.startsWith("  mkdir "))!
  await promisify(execFile)("/bin/sh", ["-c", `umask 022\n${setup}`], { env: { HOME: home } })
  for (const path of [join(home, ".chariox", "keys"), join(home, ".chariox", "keys", "app-publisher")]) {
    assert.equal((await stat(path)).mode & 0o777, 0o700)
  }
  await assert.rejects(stat(defaultAppDevKey(home)), /ENOENT/)
})

test("an existing legacy path requires explicit --key and is kept at its original location", async t => {
  const f = await fixture(t, ["dev", "app-publisher", "private"])
  const h = harness(f)
  const loop = new AppDevLoop(h.deps)
  t.after(() => loop.dispose())
  const before = await stat(f.key)
  await assert.rejects(loop.register("my-app"), /No developer signing key at .*\.chariox\/keys\/app-publisher\/private/)
  assert.equal(h.packs.length, 0)
  await loop.register("my-app", { key: relative(f.root, f.key) })
  await loop.settled()
  assert.equal(h.packs[0]!.key, f.key)
  const after = await stat(f.key)
  assert.deepEqual([after.ino, after.size, after.mtimeMs, after.mode], [before.ino, before.size, before.mtimeMs, before.mode])
  await assert.rejects(stat(defaultAppDevKey(f.home)), /ENOENT/)
})

// MP-08 / MP-10 / MP-11: registration remains a publisher-signed release path.
test("register selects the release and ignores dev copies in every Room", async t => {
  const f=await fixture(t), h=harness(f), loop=new AppDevLoop(h.deps)
  const entry=(installation_id:string,channel:string,dev_room_id:string|null)=>({installation_id,app_id:"com.example.todo",channel,dev_room_id,active_release:{},data_kept:false})
  h.deps.send=async()=>({AppInstallationsListed:{installations:[entry("registered","release",null),entry("own-dev","dev","session-1"),entry("other-dev","dev","session-2")],next_cursor:null}})
  await loop.register("my-app",{key:f.key})
  assert.deepEqual(h.calls,[{kind:"update",installation:"registered",source:"register"}])
  assert.equal(h.packs[0]!.key,f.key)
  assert.equal(loop.active,false)
  await assert.rejects(stat(h.workspaces[0]!),/ENOENT/)
})

test("ignored paths include dependency and generated Rust build trees",()=>{
  for(const file of [".DS_Store","bundle/.cache/x","node_modules/a.js","target/wasm32-wasip2/app.wasm"])assert.equal(ignored(file),true)
  assert.equal(ignored("app.ts"),false)
  assert.equal(ignored(null),false)
})

// MP-08 / MP-10 / MP-11: remote source stays on the client and uses D1 uploads.
test("remote dev watches client source and updates only its Room's dev copy", async t => {
  const f = await fixture(t), h = harness(f)
  let change!: (file: string | null) => void
  const entry = (installation_id: string, channel: string, dev_room_id: string | null) => ({ installation_id, app_id: "com.example.todo", channel, dev_room_id, active_release: {}, data_kept: false })
  h.deps.send = async request => {
    assert.ok(request.ListAppInstallations, "remote source must use the existing package upload installer")
    return { AppInstallationsListed: { installations: [entry("release", "release", null), entry("other-room", "dev", "session-2"), entry("own-dev", "dev", "session-1")], next_cursor: null } }
  }
  const loop = new AppDevLoop({ ...h.deps, isKernelLocal: () => false,
    debounceMs: 1, watch: (root, callback) => { assert.equal(root, f.app); change = callback; return { close() {} } },
  })
  t.after(() => loop.dispose())
  await loop.start("my-app"); await loop.settled()
  change("bundle/ui/index.html"); await loop.settled()
  assert.deepEqual(h.calls, [
    { kind: "update", installation: "own-dev", source: "dev_push" },
    { kind: "update", installation: "own-dev", source: "dev_push" },
  ])
  assert.equal(h.packs.length, 2)
  assert.equal(h.packs[0]!.key, undefined)
  await loop.stop()
  await assert.rejects(stat(h.workspaces[0]!), /ENOENT/)
})
