import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, writeFile, rm, rename, symlink, open } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { AppFileInstaller, FollowLostContact, formatInstallFailure, formatInstallOperation, KernelFailure } from "./app-install-file.js"
import { AppFileSource, chunkBytes, maxArchiveBytes, InstallFileChanged } from "./app-install-file/source.js"
import { handleAppSlashCommand } from "./app-command-handler.js"
import { runAppCommand } from "./app-command.js"
import { parseSlashCommand, sharedShellCommandForSlashCommand } from "./commands.js"

type Message = Record<string, any>
const digest = (bytes: Uint8Array) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(done => { resolve = done })
  return { promise, resolve }
}
async function sourceFixture(t: test.TestContext, bytes = Buffer.alloc(chunkBytes + 21, 0x61)) {
  const root = await mkdtemp(join(tmpdir(), "chariox-app-file-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const path = join(root, "local App.cxapp")
  await writeFile(path, bytes)
  return { root, path, bytes }
}
/** Stateful protocol fixture: retries preserve bytes and durable receipts. */
function kernel() {
  const requests: Message[] = []
  const handle = `upload_${"a".repeat(64)}`
  let upload: Message | undefined
  let status: Message | undefined
  let body = Buffer.alloc(0)
  let uploadId: string | undefined
  return {
    requests,
    get bytes() { return body },
    get upload() { return upload },
    get status() { return status },
    send: async (request: Message): Promise<Message> => {
      requests.push(request)
      if (request.BeginAppPackageUpload) {
        const v = request.BeginAppPackageUpload
        if (!upload || uploadId !== v.request_id) {
          uploadId = v.request_id
          upload = { handle, expected_size: v.expected_size, sha256: v.sha256, accepted_bytes: 0, phase: "receiving", expires_at_ms: Date.now() + 60_000 }
          body = Buffer.alloc(0)
        } else {
          assert.equal(v.expected_size, upload.expected_size)
          assert.equal(v.sha256, upload.sha256)
        }
      } else if (request.PutAppPackageUploadChunk) {
        const v = request.PutAppPackageUploadChunk
        assert.equal(v.handle, handle)
        assert.equal(upload?.phase, "receiving")
        const bytes = Buffer.from(v.data_base64, "base64")
        assert.ok(bytes.length <= chunkBytes)
        assert.equal(v.chunk_sha256, digest(bytes))
        if (v.offset === body.length) body = Buffer.concat([body, bytes])
        else assert.deepEqual(body.subarray(v.offset, v.offset + bytes.length), bytes)
        upload!.accepted_bytes = body.length
      } else if (request.AbortAppPackageUpload) {
        assert.equal(request.AbortAppPackageUpload.handle, handle)
        upload!.phase = "aborted"
      } else if (request.GetAppInstallation) {
        return { AppInstallation: { installation: { installation_id: request.GetAppInstallation.installation_id, app_id: "com.example.todo", generation: "7", active_release: null, pending_generation: null, admission_paused: false } } }
      } else if (request.BeginAppInstall || request.BeginAppUpdate) {
        const v = request.BeginAppInstall ?? request.BeginAppUpdate
        assert.equal(upload?.phase, "receiving")
        assert.equal(body.length, upload.expected_size)
        assert.equal(v.expected_package_digest, digest(body))
        status ??= { request_id: v.request_id, package_digest: v.expected_package_digest, phase: "preparing", installation_id: null, generation: null, interaction_id: null, failure: null }
        assert.equal(v.request_id, status.request_id)
        return { AppInstallOperationStatus: { operation: { ...status } } }
      } else if (request.GetAppInstallOperation || request.CancelAppInstallOperation) {
        const v = request.GetAppInstallOperation ?? request.CancelAppInstallOperation
        if (!status || status.request_id !== v.request_id) return { AppRequestFailed: { code: "not_found" } }
        if (request.CancelAppInstallOperation) status.phase = "cancelled"
        return { AppInstallOperationStatus: { operation: { ...status } } }
      } else assert.fail(`unexpected request ${Object.keys(request)}`)
      return { AppPackageUploadStatus: { upload: { ...upload } } }
    },
  }
}

test("held file streams bounded chunks, rejects mutation/replacement and oversized/link inputs", async t => {
  const f = await sourceFixture(t)
  const source = await AppFileSource.open(f.path, f.root, () => false, () => {})
  assert.equal(source.digest, digest(f.bytes))
  assert.equal((await source.chunk(0)).bytes.length, chunkBytes)
  assert.equal((await source.chunk(chunkBytes)).bytes.length, 21)
  await writeFile(f.path, Buffer.alloc(f.bytes.length, 0x62))
  await assert.rejects(source.chunk(0), InstallFileChanged)
  await source.close()
  const replacement = await AppFileSource.open(f.path, f.root, () => false, () => {})
  await rename(f.path, `${f.path}.old`)
  await writeFile(f.path, f.bytes)
  await assert.rejects(replacement.unchanged(), InstallFileChanged)
  await replacement.close()
  const link = join(f.root, "link.cxapp")
  await symlink(f.path, link)
  await assert.rejects(AppFileSource.open(link, f.root, () => false, () => {}))
  const huge = await open(join(f.root, "huge.cxapp"), "w")
  await huge.truncate(maxArchiveBytes + 1)
  await huge.close()
  await assert.rejects(AppFileSource.open("huge.cxapp", f.root, () => false, () => {}), /128 MiB/)
})

test("normal quoted /app install uses current session and only bytes cross the shared transport", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  const installer = new AppFileInstaller(k.send, () => {}, f.root)
  t.after(() => installer.dispose())
  const notices: string[] = []
  const command = parseSlashCommand('/app install "local App.cxapp"')!
  assert.equal(command.kind, "app")
  assert.equal(sharedShellCommandForSlashCommand(command.raw), null)
  await handleAppSlashCommand({
    sendAppRequest: k.send, appFileInstaller: installer, currentAppSessionId: () => "current-session",
    appendNotice: value => { notices.push(value) }, flashFooter: value => assert.fail(value),
  }, command as Extract<typeof command, { kind: "app" }>)
  assert.deepEqual(k.bytes, f.bytes)
  assert.equal(k.requests.find(v => v.BeginAppInstall)!.BeginAppInstall.session_id, "current-session")
  assert.ok(!JSON.stringify(k.requests).includes("local App.cxapp"))
  assert.ok(!JSON.stringify(k.requests).includes(f.root))
  assert.deepEqual([...new Set(k.requests.map(v => Object.keys(v)[0]))], ["BeginAppPackageUpload", "PutAppPackageUploadChunk", "BeginAppInstall"])
  assert.match(notices.at(-1)!, /Preparing App/)
  // Preparing still needs the upload; its durable verified stage releases that dependency.
  assert.equal(k.upload!.phase, "receiving")
  k.status!.phase = "awaiting_approval"
  assert.equal((await installer.status()).phase, "awaiting_approval")
  assert.equal(k.upload!.phase, "aborted")
  // Approved, waiting for a worker slot: a known, still active phase.
  k.status!.phase = "queued"
  assert.equal((await installer.status()).phase, "queued")
  assert.ok(!k.requests.some(v => v.RespondToInteraction))
})

test("/app install follows its operation and reports each phase through the outcome", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  const installer = new AppFileInstaller(k.send, () => {}, f.root, 5)
  t.after(() => installer.dispose())
  const notices: string[] = []
  const command = parseSlashCommand('/app install "local App.cxapp"')! as Extract<ReturnType<typeof parseSlashCommand>, { kind: "app" }>
  const deps = {
    sendAppRequest: k.send, appFileInstaller: installer, currentAppSessionId: () => "current-session",
    appendNotice: (value: string) => { notices.push(value) }, flashFooter: (value: string) => assert.fail(value),
  }
  // The command returns while the operation waits, so the prompt is free for the approval.
  await handleAppSlashCommand(deps, command)
  assert.match(notices.at(-1)!, /^Preparing App\. Operation app-install-/)
  const reached = async (pattern: RegExp) => {
    for (let wait = 0; !notices.some(notice => pattern.test(notice)); wait++) {
      assert.ok(wait < 400, `no notice matching ${pattern}: ${notices.join(" | ")}`)
      await new Promise(resolve => setTimeout(resolve, 5))
    }
  }
  k.status!.phase = "awaiting_approval"
  await reached(/^Awaiting approval/)
  // Running the same command again (as a lost connection advises) joins the follow: no second report.
  await handleAppSlashCommand(deps, command)
  k.status!.phase = "starting"
  await reached(/^Starting App\./)
  Object.assign(k.status!, { phase: "committed", installation_id: "app_1", generation: "1" })
  await reached(/^App operation complete: app_1\. Operation app-install-[^ ]+\.$/)
  const polls = k.requests.filter(v => v.GetAppInstallOperation).length
  await new Promise(resolve => setTimeout(resolve, 30))
  assert.equal(k.requests.filter(v => v.GetAppInstallOperation).length, polls, "polling stops at the outcome")
  assert.deepEqual(notices.filter(notice => notice.startsWith("App operation")), [notices.at(-1)])
  assert.equal(notices.filter(notice => notice.startsWith("Starting App")).length, 1)
  // Many polls, one release of the upload the kernel took into the operation.
  assert.ok(polls > 3)
  assert.equal(k.requests.filter(v => v.AbortAppPackageUpload).length, 1)
})

test("a followed App operation rides out busy status reads and gives up after ten in a row", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  let busy = 0
  const send = async (request: Message): Promise<Message> => {
    if (request.GetAppInstallOperation && busy > 0) { busy -= 1; throw new KernelFailure("busy") }
    return k.send(request)
  }
  const installer = new AppFileInstaller(send, () => {}, f.root)
  t.after(() => installer.dispose())
  const started = await installer.install(f.path, "current-session")
  busy = 3
  Object.assign(k.status!, { phase: "committed", installation_id: "app_1" })
  const reports: string[] = []
  assert.equal((await installer.follow(started, next => reports.push(next.phase), { pollMs: 1 })).phase, "committed")
  assert.deepEqual(reports, ["committed"])

  const lost = kernel()
  const lostSend = async (request: Message): Promise<Message> => {
    if (request.GetAppInstallOperation) throw new KernelFailure("busy")
    return lost.send(request)
  }
  const lostInstaller = new AppFileInstaller(lostSend, () => {}, f.root)
  t.after(() => lostInstaller.dispose())
  await assert.rejects(lostInstaller.follow(await lostInstaller.install(f.path, "current-session"), () => {}, { pollMs: 1 }),
    (error: unknown) => error instanceof FollowLostContact && /^Lost contact with the kernel while following App operation app-install-/.test(error.message))
})

test("a followed App operation reports its failure, and closing the terminal stops following", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  const installer = new AppFileInstaller(k.send, () => {}, f.root)
  t.after(() => installer.dispose())
  const started = await installer.install(f.path, "current-session")
  const reports: string[] = []
  const following = installer.follow(started, next => reports.push(next.phase), { pollMs: 5 })
  Object.assign(k.status!, { phase: "failed", failure: "app_install_publisher_not_enrolled" })
  assert.equal((await following).phase, "failed")
  assert.deepEqual(reports, ["failed"])
  assert.match(formatInstallOperation((await installer.status())), /^App operation failed\. This publisher must be enrolled/)

  const other = kernel()
  const closing = new AppFileInstaller(other.send, () => {}, f.root)
  const pending = await closing.install(f.path, "current-session")
  const stopped = closing.follow(pending, () => assert.fail("no report after close"), { pollMs: 60_000 })
  await closing.dispose()
  assert.equal((await stopped).phase, "preparing")
})

test("/app update fences the shared upload on the generation it read first", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  const installer = new AppFileInstaller(k.send, () => {}, f.root)
  t.after(() => installer.dispose())
  const notices: string[] = []
  const deps = {
    sendAppRequest: k.send, appFileInstaller: installer, currentAppSessionId: () => "current-session",
    appendNotice: (value: string) => { notices.push(value) }, flashFooter: (value: string) => assert.fail(value),
  }
  const parsed = parseSlashCommand('/app update todo "local App.cxapp"')!
  const command = parsed as Extract<typeof parsed, { kind: "app" }>
  assert.equal(sharedShellCommandForSlashCommand(command.raw), null)
  await assert.rejects(handleAppSlashCommand(deps, { ...command, raw: '/app update "local App.cxapp"' }), /usage: \/app update/)
  await handleAppSlashCommand(deps, command)
  assert.deepEqual(k.bytes, f.bytes)
  assert.deepEqual([...new Set(k.requests.map(v => Object.keys(v)[0]))], ["GetAppInstallation", "BeginAppPackageUpload", "PutAppPackageUploadChunk", "BeginAppUpdate"])
  const { session_id, request_id, installation_id, expected_generation } = k.requests.find(v => v.BeginAppUpdate)!.BeginAppUpdate
  assert.deepEqual([session_id, installation_id, expected_generation], ["current-session", "todo", "7"])
  assert.match(request_id, /^app-update-/)
  assert.match(notices.at(-1)!, /Preparing App/)
  await assert.rejects(installer.install(f.path, "current-session"), /Another App installation or update is retained/)
  // An ended operation has nothing left to check or cancel.
  assert.equal(formatInstallOperation({ ...k.status!, phase: "committed", installation_id: "todo" } as never), `App operation complete: todo. Operation ${request_id}.`)
  assert.equal(formatInstallOperation({ ...k.status!, phase: "starting" } as never), `Starting App. Operation ${request_id}. Use /app operation for status; /app cancel to cancel before it completes.`)
  assert.match(formatInstallOperation({ ...k.status!, phase: "failed", failure: "app_update_schema_downgrade" } as never), /^App operation failed\. This release's data schema is older than the installed App's/)
  assert.match(formatInstallOperation({ ...k.status!, phase: "queued" } as never), /^Approved; waiting for a free App worker slot to start/)
  assert.deepEqual(installer.retained(), { path: f.path, request: request_id, digest: digest(f.bytes), installation: "todo", begun: true })
  assert.equal(installer.discardRetained(), false)
  k.status!.phase = "committed"
  assert.equal((await installer.status()).phase, "committed")
  assert.equal(installer.retained(), undefined)
  assert.equal(installer.discardRetained(), true)
  await assert.rejects(installer.status(), /No App installation in this terminal/)
})

test("an update refused because another client's update is running says what that update is doing", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  let latest = { phase: "committed", generation: "7", release: { version: "1.2.5" } }
  const aborted: string[] = []
  const send = async (request: Message): Promise<Message> => {
    if (request.BeginAppUpdate) return { AppRequestFailed: { code: "conflict" } }
    if (request.GetAppInstallationJournal) return { AppInstallationJournal: { installation_id: "todo", updates: [latest] } }
    if (request.CancelAppInstallOperation) return { AppRequestFailed: { code: "not_found" } }
    if (request.AbortAppPackageUpload) aborted.push(request.AbortAppPackageUpload.handle)
    return k.send(request)
  }
  const refused = async () => {
    const installer = new AppFileInstaller(send, () => {}, f.root)
    t.after(() => installer.dispose())
    return installer.update("todo", f.path, "s1").then(() => assert.fail("refused"), (error: Error) => error.message)
  }
  // The fixture reads generation 7: the journal's generation-7 release is not the competing one.
  assert.match(await refused(), /Another client's update of this App is being prepared\. Wait for it, then check \/app status todo\./)
  latest = { phase: "quiescing", generation: "8", release: { version: "1.2.6" } }
  assert.match(await refused(), /Another update of this App is in progress: version 1\.2\.6 \(generation 8\) is quiescing/)
  latest = { phase: "committed", generation: "8", release: { version: "1.2.6" } }
  assert.match(await refused(), /Another client just updated this App to version 1\.2\.6 \(generation 8\)/)
  // The kernel refused each Begin, so each refused client aborted its upload.
  assert.equal(aborted.length, 3)
})

test("an update of an installation whose data was deleted says to install again", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  const send = async (request: Message): Promise<Message> => {
    if (request.BeginAppUpdate) return { AppRequestFailed: { code: "conflict" } }
    if (request.GetAppInstallation) return { AppInstallation: { installation: { installation_id: "todo", app_id: "com.chariox.todo",
      generation: "7", active_release: null, pending_generation: null, admission_paused: false, data_kept: false } } }
    if (request.CancelAppInstallOperation) return { AppRequestFailed: { code: "not_found" } }
    return k.send(request)
  }
  const installer = new AppFileInstaller(send, () => {}, f.root)
  t.after(() => installer.dispose())
  const message = await installer.update("todo", f.path, "s1").then(() => assert.fail("refused"), (error: Error) => error.message)
  assert.match(message, /App todo is uninstalled and its data was deleted, so nothing is left to reinstall into\. Install the App again instead\./)
})

test("a resumed attempt whose status read fails keeps the kernel operation", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  let lostBegins = 3
  let failStatus = false
  const send = async (request: Message): Promise<Message> => {
    // The kernel takes the Begin, but every reply is lost.
    if (request.BeginAppInstall && lostBegins > 0) {
      lostBegins -= 1
      await k.send(request)
      throw new Error("connection lost")
    }
    if (request.GetAppInstallOperation && failStatus) return { AppRequestFailed: { code: "storage_unavailable" } }
    return k.send(request)
  }
  const installer = new AppFileInstaller(send, () => {}, f.root)
  t.after(() => installer.dispose())
  await assert.rejects(installer.install(f.path, "s1"), /Connection interrupted/)
  failStatus = true
  await assert.rejects(installer.install(f.path, "s1"), /storage/i)
  assert.equal(k.requests.some(request => request.CancelAppInstallOperation), false)
})

test("lost chunk and install replies resume using original IDs and authoritative offset", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  let disconnected = true
  const installer = new AppFileInstaller(async request => {
    const result = await k.send(request)
    if (disconnected && request.PutAppPackageUploadChunk) throw new Error("reply lost")
    return result
  }, () => {}, f.root)
  t.after(() => installer.dispose())
  await assert.rejects(installer.install(f.path, "session"), /Connection interrupted/)
  assert.equal(k.bytes.length, chunkBytes)
  const first = k.requests[0]!.BeginAppPackageUpload.request_id
  disconnected = false
  const status = await installer.install(f.path, "session")
  assert.deepEqual(k.bytes, f.bytes)
  assert.ok(k.requests.filter(v => v.BeginAppPackageUpload).every(v => v.BeginAppPackageUpload.request_id === first))
  assert.equal(status.phase, "preparing")
  const beginCount = k.requests.filter(v => v.BeginAppInstall).length
  assert.equal((await installer.install(f.path, "session")).request_id, status.request_id)
  assert.equal(k.requests.filter(v => v.BeginAppInstall).length, beginCount)
})

test("one cancel waits for in-flight chunk before durable Abort and never begins installation", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  const entered = deferred<void>(), release = deferred<void>()
  const installer = new AppFileInstaller(async request => {
    if (request.PutAppPackageUploadChunk) { entered.resolve(); await release.promise }
    return k.send(request)
  }, () => {}, f.root)
  const transfer = installer.install(f.path, "session")
  const failed = assert.rejects(transfer, /cancelled/)
  await entered.promise
  let settled = false
  const cancel = installer.cancel().then(value => { settled = true; return value })
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(settled, false)
  assert.ok(!k.requests.some(v => v.AbortAppPackageUpload))
  release.resolve()
  await failed
  await cancel
  assert.equal(k.upload!.phase, "aborted")
  assert.ok(!k.requests.some(v => v.BeginAppInstall))
  assert.ok(k.requests.findIndex(v => v.AbortAppPackageUpload) > k.requests.findIndex(v => v.PutAppPackageUploadChunk))
  await installer.dispose()
})

test("same-length change during transfer aborts own upload without submitting changed package", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  let changed = false
  const installer = new AppFileInstaller(async request => {
    const result = await k.send(request)
    if (request.PutAppPackageUploadChunk && !changed) {
      changed = true
      await writeFile(f.path, Buffer.alloc(f.bytes.length, 0x62))
    }
    return result
  }, () => {}, f.root)
  await assert.rejects(installer.install(f.path, "session"), InstallFileChanged)
  assert.equal(k.upload!.phase, "aborted")
  assert.ok(!k.requests.some(v => v.BeginAppInstall))
  await installer.dispose()
})

test("shutdown retains the transfer through the delayed request and cleans only its pre-Begin upload", async t => {
  const f = await sourceFixture(t)
  const k = kernel()
  const entered = deferred<void>(), release = deferred<void>()
  const installer = new AppFileInstaller(async request => {
    if (request.PutAppPackageUploadChunk) { entered.resolve(); await release.promise }
    return k.send(request)
  }, () => {}, f.root)
  const transfer = installer.install(f.path, "session")
  const failed = assert.rejects(transfer, /cancelled/)
  await entered.promise
  let drained = false
  const shutdown = installer.dispose().then(() => { drained = true })
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(drained, false)
  release.resolve()
  await failed
  await shutdown
  assert.equal(k.upload!.phase, "aborted")
  assert.ok(!k.requests.some(v => v.BeginAppInstall))
})

test("lost Begin reply remains recoverable and cancellation preserves the durable receipt", async t => {
  const f = await sourceFixture(t, Buffer.from("package fixture"))
  const k = kernel()
  let disconnected = true
  const installer = new AppFileInstaller(async request => {
    const result = await k.send(request)
    if (disconnected && request.BeginAppInstall) throw new Error("reply lost")
    return result
  }, () => {}, f.root)
  await assert.rejects(installer.install(f.path, "session"), /Connection interrupted/)
  const request = k.status!.request_id
  disconnected = false
  assert.equal((await installer.status()).request_id, request)
  assert.equal((await installer.cancel())!.phase, "cancelled")
  assert.equal((await installer.status()).request_id, request)
  assert.equal(k.upload!.phase, "aborted")
  await installer.dispose()
})

test("cancelling a lost upload Begin reply recovers its original handle before Abort", async t => {
  const f = await sourceFixture(t, Buffer.from("package fixture"))
  const k = kernel()
  const entered = deferred<void>(), release = deferred<void>()
  let first = true
  const installer = new AppFileInstaller(async request => {
    const result = await k.send(request)
    if (request.BeginAppPackageUpload && first) {
      first = false
      entered.resolve()
      await release.promise
      throw new Error("lost upload handle reply")
    }
    return result
  }, () => {}, f.root)
  const failed = assert.rejects(installer.install(f.path, "session"), /cancelled/)
  await entered.promise
  const cancelling = installer.cancel()
  release.resolve()
  await failed
  await cancelling
  const begins = k.requests.filter(v => v.BeginAppPackageUpload)
  assert.ok(begins.length >= 2)
  assert.ok(begins.every(v => v.BeginAppPackageUpload.request_id === begins[0]!.BeginAppPackageUpload.request_id))
  assert.equal(k.upload!.phase, "aborted")
  assert.ok(!k.requests.some(v => v.PutAppPackageUploadChunk || v.BeginAppInstall))
  await installer.dispose()
})

test("chariox app install that outlasts its wait says only an approval needs the owner", async t => {
  for (const [phase, next] of [
    ["queued", /Still queued after waiting; .* It starts once an App worker slot is free/],
    ["awaiting_approval", /Still awaiting_approval after waiting; .* Finish it in session s1's terminal/],
    ["starting", /Still starting after waiting; .* It continues on its own/],
  ] as const) {
    const f = await sourceFixture(t)
    const k = kernel()
    const send = async (request: Message) => {
      const reply = await k.send(request)
      if (request.BeginAppInstall) k.status!.phase = phase
      return reply
    }
    await assert.rejects(runAppCommand(["app", "install", f.path, "--session", "s1"], {
      createClient: () => ({ send, close: async () => {} }),
      write: () => {},
      installWaitMs: 20,
      installPollMs: 1,
    }), (error: Error) => {
      assert.match(error.message, next)
      if (phase !== "awaiting_approval") assert.doesNotMatch(error.message, /Finish it in session/)
      return true
    })
  }
})

test("each stable package error code renders its own message (V-PKG-03)", () => {
  const codes = [
    "invalid_arguments", "io", "invalid_developer_key", "invalid_archive", "archive_limit", "invalid_path",
    "duplicate_path", "invalid_manifest", "invalid_schema", "incompatible_protocol", "incompatible_sdk",
    "incompatible_contract", "incompatible_resource_policy", "untrusted_publisher", "invalid_signature",
    "integrity_mismatch", "missing_entry", "unexpected_entry", "unsupported_feature",
  ]
  const rendered = codes.map((code) => formatInstallFailure(`app_install_package_${code}`))
  assert.equal(new Set(rendered).size, codes.length, "every code reads differently")
  for (const message of rendered) assert.doesNotMatch(message, /^Kernel failure:|could not complete/)
  assert.deepEqual(
    Object.fromEntries(codes.map((code, index) => [code, rendered[index]])),
    {
      invalid_arguments: "The package request was invalid.",
      io: "The kernel could not read the package.",
      invalid_developer_key: "The package's developer key is invalid.",
      invalid_archive: "The file is not a valid .cxapp archive.",
      archive_limit: "The package exceeds the archive size or file-count limits.",
      invalid_path: "The package contains an invalid file path.",
      duplicate_path: "The package contains the same file path twice.",
      invalid_manifest: "The package manifest is invalid.",
      invalid_schema: "A tool, event or state schema in the package is invalid.",
      incompatible_protocol: "The App needs a kernel protocol version this kernel does not support.",
      incompatible_sdk: "The App was built with an SDK version this kernel does not support.",
      incompatible_contract: "The App's declared contract is not supported by this kernel.",
      incompatible_resource_policy: "The App requests more resources than this kernel allows.",
      untrusted_publisher: "The package's publisher is not trusted by this kernel.",
      invalid_signature: "The package signature is invalid.",
      integrity_mismatch: "The package contents do not match its signed digest.",
      missing_entry: "The package is missing a file its manifest declares.",
      unexpected_entry: "The package contains a file its manifest does not declare.",
      unsupported_feature: "The App uses a feature this kernel does not support.",
    },
  )
})

test("release and upload failures render their own messages; inherited keys fall back", () => {
  const codes = ["invalid_request", "upload_aborted", "upload_digest_mismatch", "release_limit", "release_unsafe", "release_archive_mismatch"]
  const rendered = codes.map((code) => formatInstallFailure(`app_install_${code}`))
  assert.equal(new Set(rendered).size, codes.length)
  for (const message of rendered) assert.doesNotMatch(message, /^Kernel failure:|could not complete/)
  for (const key of ["constructor", "toString", "__proto__"]) assert.equal(formatInstallFailure(key), "The kernel could not complete the App operation.")
})

// Both standalone CLI and /app in the TUI use this operation formatter.
test("lifecycle update failures retain their kernel cause in operation output", () => {
  const preparation = formatInstallFailure("app_lifecycle_notification")
  assert.match(preparation, /did not complete preparation/)
  assert.match(preparation, /installed version is unchanged/)
  assert.match(formatInstallOperation({ request_id: "lifecycle-update", phase: "failed", installation_id: "installed", generation: "2", package_digest: `sha256:${"a".repeat(64)}`, interaction_id: null, failure: "app_lifecycle_notification" }), /app_lifecycle_notification/)
  for (const failure of ["app_lifecycle_registration", "app_lifecycle_health", "app_lifecycle_preparation"]) {
    assert.equal(formatInstallFailure(failure), `Kernel failure: ${failure}.`)
    assert.match(formatInstallOperation({ request_id: "lifecycle-update", phase: "failed", installation_id: "installed", generation: "2", package_digest: `sha256:${"a".repeat(64)}`, interaction_id: null, failure }), new RegExp(failure))
  }
  assert.equal(formatInstallFailure("app_lifecycle_bad\nprivate details"), "The kernel could not complete the App operation.")
})

// Shared by standalone CLI operation output and TUI /app operation notices.
test("readiness cancellation and deadline stay distinct in operation output", () => {
  for (const suffix of ["cancelled", "deadline"]) {
    const failure = `app_lifecycle_registration_${suffix}`
    const text = formatInstallOperation({ request_id: "readiness", phase: "failed", installation_id: "installed", generation: "1", package_digest: `sha256:${"a".repeat(64)}`, interaction_id: null, failure })
    assert.match(text, new RegExp(failure))
    if (suffix === "cancelled") assert.doesNotMatch(text, /deadline|timed out|timeout/)
    else assert.doesNotMatch(text, /cancelled/)
  }
})
