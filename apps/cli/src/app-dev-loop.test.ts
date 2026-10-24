import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdir, mkdtemp, realpath, rm, stat, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, relative } from "node:path"
import test from "node:test"
import { promisify } from "node:util"
import { AppDevLoop, defaultAppDevKey, ignored, type AppDevDeps, type AppDevPackOptions } from "./app-dev-loop.js"
import { AppFileInstaller } from "./app-install-file.js"
import { handleAppSlashCommand } from "./app-command-handler.js"
import { parseSlashCommand, sharedShellCommandForSlashCommand } from "./commands.js"

type Message = Record<string, any>
type Status = { request_id: string; phase: string; installation_id: string | null; generation: string | null; package_digest: string; interaction_id: null; failure: string | null }
function deferred<T = void>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(done => { resolve = done })
  return { promise, resolve }
}

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

/** Fake watcher, packer, installer and kernel list; no real watcher, helper or kernel runs. */
function harness(f: { root: string, app: string, home: string, key: string }, options: { installed?: boolean, failure?: string } = {}) {
  const notices: string[] = []
  const packs: AppDevPackOptions[] = []
  const calls: { kind: "install" | "update", installation?: string, path: string, session: string }[] = []
  const requests: Message[] = []
  let onChange: ((file: string | null) => void) | undefined
  let closed = 0
  let version = 0
  let generation = 7
  let installed = options.installed ?? false
  let gate: Promise<void> | undefined
  const workspaces: string[] = []
  const statuses = new Map<string, Status>()
  const operate = (kind: "install" | "update", path: string): Status => {
    const request_id = `app-${kind}-${calls.length}`
    const status: Status = { request_id, phase: "preparing", installation_id: kind === "update" ? "todo" : null, generation: null, package_digest: `sha256:${"a".repeat(64)}`, interaction_id: null, failure: null }
    statuses.set(request_id, status)
    assert.ok(path.endsWith(".cxapp"))
    return { ...status }
  }
  const deps: AppDevDeps = {
    cwd: f.root, home: f.home, debounceMs: 5, pollMs: 1,
    notice: message => { notices.push(message) },
    currentSession: () => "session-1",
    workspace: async () => { const dir = await mkdtemp(join(f.root, "work-")); workspaces.push(dir); return dir },
    watch: (directory, change) => {
      assert.equal(directory, f.app)
      onChange = change
      return { close: () => { closed++ } }
    },
    pack: async (value) => {
      packs.push(value)
      await gate
      await writeFile(value.output, "package")
      version++
      return { appId: "com.example.todo", version: `1.0.${version}`, packageDigest: `sha256:${String(version).repeat(64)}` }
    },
    send: async (request) => {
      requests.push(request)
      assert.ok((request as Message).ListAppInstallations)
      if (!(request as Message).ListAppInstallations.after) {
        return { AppInstallationsListed: { installations: [{ installation_id: "other", app_id: "com.example.other", generation: "1", active_release: {}, pending_generation: null, admission_paused: false }], next_cursor: "other" } }
      }
      return { AppInstallationsListed: { installations: installed ? [{ installation_id: "todo", app_id: "com.example.todo", generation: String(generation), active_release: {}, pending_generation: null, admission_paused: false }] : [], next_cursor: null } }
    },
    installer: {
      install: async (path, session) => { calls.push({ kind: "install", path, session }); return operate("install", path) as never },
      update: async (installation, path, session) => { calls.push({ kind: "update", installation, path, session }); return operate("update", path) as never },
      status: async (request) => {
        const status = statuses.get(request!)!
        if (options.failure) Object.assign(status, { phase: "failed", failure: options.failure })
        else { generation++; installed = true; Object.assign(status, { phase: "committed", installation_id: "todo", generation: String(generation) }) }
        return { ...status } as never
      },
      cancel: async () => assert.fail("nothing retained"),
      retained: () => undefined,
      discardRetained: () => false,
    },
  }
  return {
    deps, notices, packs, calls, requests, workspaces,
    change: (file: string | null = "bundle/ui/index.html") => onChange!(file),
    get closed() { return closed },
    hold() { const d = deferred(); gate = d.promise; return () => { gate = undefined; d.resolve() } },
  }
}

test("the default signing path uses the protected publisher namespace in the selected HOME", async t => {
  const f = await fixture(t)
  assert.equal(defaultAppDevKey(f.home), join(f.home, ".chariox", "keys", "app-publisher", "private"))
  const h = harness(f)
  const loop = new AppDevLoop(h.deps)
  t.after(() => loop.dispose())
  assert.deepEqual(await loop.start("my-app"), { directory: f.app, key: f.key })
  await loop.settled()
  assert.equal(h.packs[0]!.key, f.key)
})

test("missing-key guidance uses protected paths and private directory modes without creating a key", async t => {
  const f = await fixture(t)
  const home = join(f.root, "empty-home")
  await mkdir(home)
  const h = harness(f)
  const loop = new AppDevLoop({ ...h.deps, home })
  let instructions = ""
  await assert.rejects(loop.start("my-app"), (error: Error) => {
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
  await assert.rejects(loop.start("my-app"), /No developer signing key at .*\.chariox\/keys\/app-publisher\/private/)
  assert.equal(h.packs.length, 0)
  assert.deepEqual(await loop.start("my-app", { key: relative(f.root, f.key) }), { directory: f.app, key: f.key })
  await loop.settled()
  assert.equal(h.packs[0]!.key, f.key)
  const after = await stat(f.key)
  assert.deepEqual([after.ino, after.size, after.mtimeMs, after.mode], [before.ino, before.size, before.mtimeMs, before.mode])
  await assert.rejects(stat(defaultAppDevKey(f.home)), /ENOENT/)
})

test("first cycle packs outside the App directory and installs; later changes update", async t => {
  const f = await fixture(t)
  const h = harness(f)
  const loop = new AppDevLoop(h.deps)
  t.after(() => loop.dispose())
  await assert.rejects(new AppDevLoop({ ...h.deps, home: join(f.root, "no-home") }).start("my-app"), /No developer signing key at .*app-publisher\/private[\s\S]*chariox app keygen/)
  await assert.rejects(loop.start("missing", { key: f.key }), /App directory not found/)
  await loop.start("my-app", { key: f.key })
  await loop.settled()
  assert.equal(h.packs.length, 1)
  const { output, bundle, manifest, key } = h.packs[0]!
  assert.deepEqual([bundle, manifest, key], [join(f.app, "bundle"), join(f.app, "app.json"), f.key])
  assert.ok(!output.startsWith(f.app))
  assert.deepEqual(h.calls.map(v => [v.kind, v.installation, v.session]), [["install", undefined, "session-1"]])
  await assert.rejects(stat(output), /ENOENT/)
  assert.deepEqual(h.notices.slice(1), [
    `Packed com.example.todo 1.0.1 (sha256:111111111111…)`, "Installed todo at generation 8.", "Open its view with /app open todo",
  ])
  // Paging reaches the installation after the first page.
  h.change()
  await loop.settled()
  assert.deepEqual(h.calls.map(v => [v.kind, v.installation]), [["install", undefined], ["update", "todo"]])
  assert.equal(h.notices.at(-1), "Updated todo to generation 9; App data is kept. Reopen views opened earlier with /app open todo")
  assert.deepEqual(h.requests.slice(-2), [{ ListAppInstallations: { after: null, limit: 100 } }, { ListAppInstallations: { after: "other", limit: 100 } }])
  h.change("node_modules/x/index.js")
  h.change(".git/HEAD")
  await loop.settled()
  assert.equal(h.packs.length, 2)
})

test("changes during a cycle coalesce into exactly one more cycle", async t => {
  const f = await fixture(t)
  const h = harness(f, { installed: true })
  const loop = new AppDevLoop(h.deps)
  t.after(() => loop.dispose())
  const release = h.hold()
  await loop.start(f.app, { key: f.key })
  for (let index = 0; index < 5; index++) { h.change(); await new Promise(resolve => setTimeout(resolve, 8)) }
  assert.equal(h.packs.length, 1)
  release()
  await loop.settled()
  assert.equal(h.packs.length, 2)
  assert.deepEqual(h.calls.map(v => v.kind), ["update", "update"])
})

test("stop closes the watcher and the private workspace; a new start replaces the previous loop", async t => {
  const f = await fixture(t)
  const h = harness(f, { installed: true })
  const loop = new AppDevLoop(h.deps)
  await loop.start(f.app, { key: f.key })
  await loop.settled()
  const release = h.hold()
  await loop.start(f.app, { key: f.key })
  assert.equal(h.closed, 1)
  await assert.rejects(stat(h.workspaces[0]!), /ENOENT/)
  const parsed = parseSlashCommand("/app dev stop")!
  assert.equal(sharedShellCommandForSlashCommand(parsed.raw), null)
  const notices: string[] = []
  const deps = { sendAppRequest: h.deps.send, appDevLoop: loop, appendNotice: (value: string) => { notices.push(value) }, flashFooter: (value: string) => assert.fail(value) }
  const stopping = handleAppSlashCommand(deps, parsed as Extract<typeof parsed, { kind: "app" }>)
  release()
  await stopping
  assert.equal(h.closed, 2)
  assert.equal(loop.active, false)
  h.change()
  await new Promise(resolve => setTimeout(resolve, 20))
  assert.equal(h.packs.length, 2)
  assert.equal(h.calls.length, 1)
  await assert.rejects(stat(h.workspaces[1]!), /ENOENT/)
  await handleAppSlashCommand(deps, parsed as Extract<typeof parsed, { kind: "app" }>)
  assert.deepEqual(notices, ["App dev loop stopped.", "No App dev loop is running in this terminal."])
  await assert.rejects(handleAppSlashCommand(deps, { kind: "app", raw: "/app dev", args: ["dev"] }), /usage: \/app dev/)
})

test("a failed update reports the friendly kernel failure and keeps watching", async t => {
  const f = await fixture(t)
  const h = harness(f, { installed: true, failure: "app_update_schema_downgrade" })
  const loop = new AppDevLoop(h.deps)
  t.after(() => loop.dispose())
  await loop.start(f.app, { key: f.key })
  await loop.settled()
  assert.match(h.notices.at(-1)!, /^Update failed: This release's data schema is older than the installed App's; App data is never migrated to an older schema\. Operation app-update-1\.$/)
  h.deps.pack = async () => { throw new Error("INVALID_MANIFEST: app.json is invalid") }
  h.change()
  await loop.settled()
  assert.equal(h.notices.at(-1), "App dev: INVALID_MANIFEST: app.json is invalid")
  assert.equal(loop.active, true)
})

/** Real installer over a kernel fixture whose connection drops after it has committed a Begin. */
async function lostBegin(t: test.TestContext, installed: boolean) {
  const f = await fixture(t)
  const requests: Message[] = []
  const ops = new Map<string, Message>()
  const uploads = new Map<string, Message>()
  let generation = 7
  let offline = false
  const installation = () => ({ installation_id: "todo", app_id: "com.example.todo", generation: String(generation), active_release: {}, pending_generation: null, admission_paused: false })
  const send = async (request: Message): Promise<Message> => {
    if (offline) throw new Error("socket closed")
    requests.push(request)
    if (request.ListAppInstallations) return { AppInstallationsListed: { installations: installed ? [installation()] : [], next_cursor: null } }
    if (request.GetAppInstallation) return { AppInstallation: { installation: installation() } }
    if (request.BeginAppPackageUpload) {
      const v = request.BeginAppPackageUpload
      if (!uploads.has(v.request_id)) uploads.set(v.request_id, { handle: `upload_${String(uploads.size).padStart(64, "0")}`, expected_size: v.expected_size, sha256: v.sha256, accepted_bytes: 0, phase: "receiving", expires_at_ms: Date.now() + 60_000 })
      return { AppPackageUploadStatus: { upload: { ...uploads.get(v.request_id) } } }
    }
    const upload = [...uploads.values()].find(value => value.handle === (request.PutAppPackageUploadChunk ?? request.AbortAppPackageUpload)?.handle)
    if (request.PutAppPackageUploadChunk) { upload!.accepted_bytes += Buffer.from(request.PutAppPackageUploadChunk.data_base64, "base64").length; return { AppPackageUploadStatus: { upload: { ...upload } } } }
    if (request.AbortAppPackageUpload) { upload!.phase = "aborted"; return { AppPackageUploadStatus: { upload: { ...upload } } } }
    const v = request.BeginAppInstall ?? request.BeginAppUpdate ?? request.GetAppInstallOperation
    if (request.BeginAppInstall || request.BeginAppUpdate) {
      generation++; installed = true
      ops.set(v.request_id, { request_id: v.request_id, phase: "committed", installation_id: "todo", generation: String(generation), package_digest: v.expected_package_digest, interaction_id: null, failure: null })
      if (ops.size === 1) { offline = true; throw new Error("socket closed") }
    }
    const op = ops.get(v.request_id)
    return op ? { AppInstallOperationStatus: { operation: { ...op } } } : { AppRequestFailed: { code: "not_found" } }
  }
  const installer = new AppFileInstaller(send, () => {}, f.root)
  const notices: string[] = []
  const packed: string[] = []
  let onChange: (() => void) | undefined
  const loop = new AppDevLoop({
    cwd: f.root, home: f.home, debounceMs: 5, pollMs: 1, send, installer, notice: value => { notices.push(value) }, currentSession: () => "session-1",
    workspace: () => mkdtemp(join(f.root, "work-")),
    watch: (_directory, change) => { onChange = () => change("bundle/ui/app.js"); return { close() {} } },
    pack: async value => {
      const bytes = Buffer.from(`package ${packed.length + 1}`)
      await writeFile(value.output, bytes)
      packed.push(`sha256:${createHash("sha256").update(bytes).digest("hex")}`)
      return { appId: "com.example.todo", version: `1.0.${packed.length}`, packageDigest: packed.at(-1)! }
    },
  })
  t.after(async () => { await loop.dispose(); await installer.dispose() })
  await loop.start("my-app", { key: f.key })
  await loop.settled()
  assert.match(notices.at(-1)!, /^App dev: Connection interrupted/)
  assert.ok(installer.retained()?.begun)
  offline = false
  onChange!()
  await loop.settled()
  const begins = requests.filter(value => value.BeginAppInstall || value.BeginAppUpdate).map(value => value.BeginAppInstall ? ["install", value.BeginAppInstall.expected_package_digest] : ["update", value.BeginAppUpdate.expected_package_digest])
  return { notices, packed, begins, retained: installer.retained() }
}

test("after a lost update Begin, the next cycle reports it as the previous build and uploads the new pack", async t => {
  const r = await lostBegin(t, true)
  assert.deepEqual(r.begins, [["update", r.packed[0]], ["update", r.packed[1]]])
  assert.equal(r.retained, undefined)
  assert.deepEqual(r.notices.slice(-4), [
    `Previous build (${r.packed[0]!.slice(0, 19)}…): Updated todo to generation 8; App data is kept.`, "Open its view with /app open todo",
    `Packed com.example.todo 1.0.2 (${r.packed[1]!.slice(0, 19)}…)`, "Updated todo to generation 9; App data is kept. Reopen views opened earlier with /app open todo",
  ])
})

test("after a lost first install that the kernel committed, the next cycle updates that installation", async t => {
  const r = await lostBegin(t, false)
  assert.deepEqual(r.begins, [["install", r.packed[0]], ["update", r.packed[1]]])
  assert.equal(r.retained, undefined)
  assert.deepEqual(r.notices.slice(-4), [
    `Previous build (${r.packed[0]!.slice(0, 19)}…): Installed todo at generation 8.`, "Open its view with /app open todo",
    `Packed com.example.todo 1.0.2 (${r.packed[1]!.slice(0, 19)}…)`, "Updated todo to generation 9; App data is kept. Reopen views opened earlier with /app open todo",
  ])
})

test("ignored paths are dotfiles and dependency trees", () => {
  assert.equal(ignored(".DS_Store"), true)
  assert.equal(ignored("bundle/.cache/x"), true)
  assert.equal(ignored("node_modules/a.js"), true)
  assert.equal(ignored("bundle/ui/index.html"), false)
  assert.equal(ignored(null), false)
})
