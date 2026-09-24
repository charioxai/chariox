import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, writeFile, rm, rename, symlink, open } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { AppFileInstaller } from "./app-install-file.js"
import { AppFileSource, chunkBytes, maxArchiveBytes, InstallFileChanged } from "./app-install-file/source.js"
import { handleAppSlashCommand } from "./app-command-handler.js"
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
      } else if (request.BeginAppInstall) {
        const v = request.BeginAppInstall
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
  assert.ok(!k.requests.some(v => v.RespondToInteraction))
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
