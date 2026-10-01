import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { runInNewContext } from "node:vm"
import { dirname } from "node:path"
import { capturePrivateHomeArchive, HOME_ARCHIVE_MINIMUM_FREE_BYTES, homeArchiveMetadataMatches } from "../apps/kernel/slice-linux-docker/managed-home-archive-stream.mjs"

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), "chariox-private-archive-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const destination = join(root, "home.tar.zst")
  const run = (script, options = {}) => capturePrivateHomeArchive({
    command: process.execPath, args: ["-e", script], env: { PATH: process.env.PATH },
    destination, maxBytes: 1024 * 1024, minimumFreeBytes: 0, timeoutMs: 5000, ...options,
  })
  return { root, destination, run }
}

test("binary home bytes stream to an exclusive private file with an exact digest", async t => {
  const { destination, run } = await fixture(t)
  const expected = Buffer.alloc(200_000); for (let i = 0; i < expected.length; i++) expected[i] = i % 256
  const oldMask = process.umask(0)
  let result
  try { result = await run('const b=Buffer.alloc(200000);for(let i=0;i<b.length;i++)b[i]=i%256;process.stdout.write(b)') }
  finally { process.umask(oldMask) }
  assert.deepEqual(await readFile(destination), expected)
  assert.deepEqual(result, { sizeBytes: expected.length, sha256: createHash("sha256").update(expected).digest("hex") })
  assert.equal((await stat(destination)).mode & 0o777, 0o600)
})

for (const [name, script, options, message] of [
  ["empty output", "", {}, /empty/],
  ["failed producer after private bytes", 'process.stdout.write("synthetic-private-data");process.stderr.write("never-return-this-secret");process.exitCode=7', {}, /failed to stream/],
  ["byte budget", 'process.stdout.write(Buffer.alloc(100000));setInterval(()=>{},1000)', { maxBytes: 1234 }, /size limit/],
  ["deadline after partial bytes", 'process.stdout.write("private");setInterval(()=>{},1000)', { timeoutMs: 150 }, /timed out/],
]) {
  test(`${name} removes partial archive and settles producer without exposing bytes`, async t => {
    const { root, run } = await fixture(t)
    await assert.rejects(run(script, options), error => {
      assert.match(error.message, message)
      assert.doesNotMatch(error.message, /synthetic-private-data|never-return-this-secret/)
      return true
    })
    assert.deepEqual(await readdir(root), [])
  })
}

test("missing producer cleans only its newly created file", async t => {
  const { root, run } = await fixture(t)
  await assert.rejects(run("", { command: join(root, "absent-program") }))
  assert.deepEqual(await readdir(root), [])
})

test("existing archive and symlink targets are never overwritten", async t => {
  const { root, destination, run } = await fixture(t)
  await writeFile(destination, "retained-generation", { mode: 0o600 })
  await assert.rejects(run('process.stdout.write("replacement")'), /EEXIST/)
  assert.equal(await readFile(destination, "utf8"), "retained-generation")
  const link = join(root, "alias")
  await symlink(destination, link)
  await assert.rejects(run('process.stdout.write("replacement")', { destination: link }), /EEXIST/)
  assert.equal(await readFile(destination, "utf8"), "retained-generation")
})

test("reserve exhaustion refuses before creating state or launching the producer", async t => {
  const { root, run } = await fixture(t)
  await assert.rejects(run('throw new Error("must not start")', { minimumFreeBytes: Number.MAX_SAFE_INTEGER }), /insufficient space/)
  assert.deepEqual(await readdir(root), [])
})

for (const options of [{ maxBytes: 0 }, { maxBytes: Infinity }, { minimumFreeBytes: -1 }, { timeoutMs: 0 }]) {
  test(`invalid capture limits refuse before any file ${JSON.stringify(options)}`, async t => {
    const { root, run } = await fixture(t)
    await assert.rejects(run("", options), /invalid.*limits/)
    assert.deepEqual(await readdir(root), [])
  })
}


test("ordinary-compatible archive capture has no implicit byte ceiling", async t => {
  const { destination, run } = await fixture(t)
  const result = await run('process.stdout.write(Buffer.alloc(4097, 7))', {
    maxBytes: undefined, timeoutMs: undefined,
  })
  assert.equal(result.sizeBytes, 4097)
  assert.equal((await readFile(destination)).length, 4097)
})

test("healthy archive progress survives the former ten-minute whole-stream deadline", async t => {
  const { root, destination, run } = await fixture(t)
  const marker = join(root, "producer-started")
  const release = join(root, "release-producer")
  const originalTimeout = globalThis.setTimeout
  const pause = ms => new Promise(resolve => originalTimeout(resolve, ms))
  t.mock.timers.enable({ apis: ["setTimeout"] })
  const capture = run(`const fs=require('node:fs');fs.writeFileSync(${JSON.stringify(marker)},'started');process.stdout.write('first');const timer=setInterval(()=>{if(fs.existsSync(${JSON.stringify(release)})){clearInterval(timer);process.stdout.write('last')}},10)`, { timeoutMs: undefined })
  const observed = capture.then(value => ({ value }), error => ({ error }))
  for (let i = 0; i < 100; i++) {
    try { await stat(marker); break } catch { await pause(10) }
  }
  await stat(marker)
  t.mock.timers.tick(10 * 60_000 + 1)
  await pause(30)
  await writeFile(release, "release")
  const result = await observed
  if (result.error) throw result.error
  assert.equal(result.value.sizeBytes, 9)
  assert.equal(await readFile(destination, "utf8"), "firstlast")
})


test("ordinary and managed archive policy retain the shared two-GiB reserve", () => {
  assert.equal(HOME_ARCHIVE_MINIMUM_FREE_BYTES, 2 * 1024 ** 3)
})

test("active disk pressure settles producer, discards only partial, and retains prior generation", async t => {
  const { root, destination, run } = await fixture(t)
  const prior = join(root, "previous.tar.zst")
  const pid = join(root, "producer-pid")
  await writeFile(prior, "retained-generation", { mode: 0o600 })
  let checks = 0
  await assert.rejects(run(`require('node:fs').writeFileSync(${JSON.stringify(pid)},String(process.pid));process.stdout.write('first');setTimeout(()=>process.stdout.write('last'),40);setInterval(()=>{},1000)`, {
    maxBytes: undefined, timeoutMs: undefined, minimumFreeBytes: 32,
    availableBytes: () => BigInt(++checks < 3 ? 4096 : 0),
  }), /insufficient space/)
  assert.equal(await readFile(prior, "utf8"), "retained-generation")
  await assert.rejects(stat(destination), { code: "ENOENT" })
  const producer = Number(await readFile(pid, "utf8"))
  assert.throws(() => process.kill(producer, 0), { code: "ESRCH" })
  assert.equal(checks, 3)
})


test("archive verification accepts large safe sizes while binding identity and exact file size", () => {
  const size = 33 * 1024 ** 3 + 1
  const metadata = { schemaVersion: 1, scope: "state", id: "owned", sizeBytes: size, sha256: "a".repeat(64) }
  assert.equal(homeArchiveMetadataMatches(metadata, "state", "owned", size), true)
  for (const changed of [{ schemaVersion: 2 }, { scope: "backup" }, { id: "foreign" }, { sizeBytes: 0 },
    { sizeBytes: -1 }, { sizeBytes: Number.MAX_SAFE_INTEGER + 1 }, { sizeBytes: 1.5 }, { sha256: "A".repeat(64) }]) {
    assert.equal(homeArchiveMetadataMatches({ ...metadata, ...changed }, "state", "owned", size), false)
  }
  assert.equal(homeArchiveMetadataMatches(metadata, "state", "owned", size + 1), false)
})


test("actual broker verification delegates safe large metadata to hashing without a wall-time ceiling", async () => {
  const source = await readFile(new URL("../apps/kernel/slice-linux-docker/managed-docker-broker.mjs", import.meta.url), "utf8")
  const body = source.slice(source.indexOf("function inspectManagedHomeArchive("), source.indexOf("\nfunction verifyManagedHomeArchive("))
  const metadata = { schemaVersion: 1, scope: "state", id: "owned", sizeBytes: 33 * 1024 ** 3 + 1, sha256: "a".repeat(64) }
  let hashes = 0
  // IO is synthetic: exercise the actual broker policy and hash argv without
  // allocating, reading, or copying a multi-GiB archive.
  const context = {
    managedHomeArchiveCoordinates: () => ({ candidate: "/private/archive", relative: ["states", "owned", "generation-abcdef", "home.tar.zst"] }),
    pinnedSharedPath: path => ({ path, fd: 7 }), dirname, join, homeArchiveMetadataMatches,
    readdirSync: () => ["home.tar.zst", "metadata.json"], readFileSync: () => JSON.stringify(metadata),
    fstatSync: () => ({ size: metadata.sizeBytes }), closeSync: () => {}, MAX_HOME_ARCHIVE_BYTES: 32 * 1024 ** 3,
    exactKeys: (value, keys) => assert.equal(Object.keys(value).sort().join(","), Array.from(keys).sort().join(",")),
    fail: message => { throw new Error(message) },
    spawnSync: (command, args, options) => {
      assert.equal(command, "/usr/bin/sha256sum")
      assert.equal(options.timeout, undefined)
      assert.equal(args[1], "/private/archive")
      hashes++
      return { status: 0, stdout: metadata.sha256 + "  /private/archive\n" }
    },
  }
  const inspect = runInNewContext("(" + body + ")", context)
  assert.equal(inspect("/private/archive").metadata.sizeBytes, metadata.sizeBytes)
  assert.equal(hashes, 1)
  metadata.sizeBytes = Number.MAX_SAFE_INTEGER + 1
  assert.throws(() => inspect("/private/archive"), /metadata is invalid/)
})
