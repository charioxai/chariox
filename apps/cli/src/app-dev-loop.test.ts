import assert from "node:assert/strict"
import { mkdir, mkdtemp, realpath, rm, stat, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { AppDevLoop, ignored, type AppDevDeps, type AppDevPackOptions } from "./app-dev-loop.js"
import { handleAppSlashCommand } from "./app-command-handler.js"
import { parseSlashCommand, sharedShellCommandForSlashCommand } from "./commands.js"

type Message = Record<string, any>
type Status = { request_id: string; phase: string; installation_id: string | null; generation: string | null; package_digest: string; interaction_id: null; failure: string | null }
function deferred<T = void>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(done => { resolve = done })
  return { promise, resolve }
}

async function fixture(t: test.TestContext) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "chariox-app-dev-test-")))
  t.after(() => rm(root, { recursive: true, force: true }))
  const app = join(root, "my-app")
  await mkdir(join(app, "bundle"), { recursive: true })
  await writeFile(join(app, "app.json"), "{}")
  await mkdir(join(root, "keys"), { mode: 0o700 })
  const key = join(root, "keys", "private")
  await writeFile(join(key), Buffer.alloc(32), { mode: 0o600 })
  return { root, app, key }
}

/** Fake watcher, packer, installer and kernel list; no real watcher, helper or kernel runs. */
function harness(f: { root: string, app: string, key: string }, options: { installed?: boolean, failure?: string } = {}) {
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
    cwd: f.root, home: join(f.root, "home"), debounceMs: 5, pollMs: 1,
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
    },
  }
  return {
    deps, notices, packs, calls, requests, workspaces,
    change: (file: string | null = "bundle/ui/index.html") => onChange!(file),
    get closed() { return closed },
    hold() { const d = deferred(); gate = d.promise; return () => { gate = undefined; d.resolve() } },
  }
}

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
  const h = harness(f, { installed: true, failure: "app_update_migration_required" })
  const loop = new AppDevLoop(h.deps)
  t.after(() => loop.dispose())
  await loop.start(f.app, { key: f.key })
  await loop.settled()
  assert.match(h.notices.at(-1)!, /^Update failed: This release changes the App's data schema; updating with data migrations is not supported yet\. Operation app-update-1\.$/)
  h.deps.pack = async () => { throw new Error("INVALID_MANIFEST: app.json is invalid") }
  h.change()
  await loop.settled()
  assert.equal(h.notices.at(-1), "App dev: INVALID_MANIFEST: app.json is invalid")
  assert.equal(loop.active, true)
})

test("ignored paths are dotfiles and dependency trees", () => {
  assert.equal(ignored(".DS_Store"), true)
  assert.equal(ignored("bundle/.cache/x"), true)
  assert.equal(ignored("node_modules/a.js"), true)
  assert.equal(ignored("bundle/ui/index.html"), false)
  assert.equal(ignored(null), false)
})
