import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { runInNewContext } from "node:vm"
import { dirname } from "node:path"
import { capturePrivateHomeArchive, HOME_ARCHIVE_MINIMUM_FREE_BYTES, HOME_ARCHIVE_PROGRESS_TIMEOUT_MS, homeArchiveMetadataMatches } from "../apps/kernel/slice-linux-docker/managed-home-archive-stream.mjs"

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

// Mocked clocks must still settle owned producers when an earlier assertion
// fails, before test state is discarded or the clock is reset.
function settleMockedCaptureAfterTest(t, observed, isSettled, pause, marker) {
  t.after(async () => {
    t.mock.timers.tick(HOME_ARCHIVE_PROGRESS_TIMEOUT_MS + 1)
    await pause(30)
    if (!isSettled() && marker) {
      try {
        const pid = Number(await readFile(marker, "utf8"))
        if (Number.isSafeInteger(pid) && pid > 0) process.kill(-pid, "SIGKILL")
      } catch {}
    }
    await observed
  })
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

for (const options of [{ maxBytes: 0 }, { maxBytes: Infinity }, { minimumFreeBytes: -1 }, { timeoutMs: 0 }, { progressTimeoutMs: 0 }, { progressTimeoutMs: 2_147_483_648 }]) {
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
  const release = join(root, "progress")
  const originalTimeout = globalThis.setTimeout
  const pause = ms => new Promise(resolve => originalTimeout(resolve, ms))
  t.mock.timers.enable({ apis: ["setTimeout"] })
  const script = "const fs=require('node:fs');let n=0;const timer=setInterval(()=>{if(fs.existsSync(" + JSON.stringify(release) + ")){const next=Number(fs.readFileSync(" + JSON.stringify(release) + ",'utf8'));if(next>n){n=next;process.stdout.write('x');if(n===4)clearInterval(timer)}}},10)"
  const capture = run(script, { timeoutMs: undefined })
  let settled = false
  const observed = capture.then(value => { settled = true; return { value } },
    error => { settled = true; return { error } })
  settleMockedCaptureAfterTest(t, observed, () => settled, pause)
  for (let n = 1; n <= 4; n++) {
    await writeFile(release, String(n))
    for (let i = 0; i < 100 && (await stat(destination)).size < n; i++) await pause(10)
    assert.equal((await stat(destination)).size, n)
    t.mock.timers.tick(240_000)
    await pause(10)
  }
  const result = await observed
  if (result.error) throw result.error
  assert.equal(result.value.sizeBytes, 4)
  assert.equal(await readFile(destination, "utf8"), "xxxx")
})


test("ordinary and managed archive policy retain the shared two-GiB reserve", () => {
  assert.equal(HOME_ARCHIVE_MINIMUM_FREE_BYTES, 2 * 1024 ** 3)
  assert.equal(HOME_ARCHIVE_PROGRESS_TIMEOUT_MS, 300_000)
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
  const body = source.slice(source.indexOf("async function inspectManagedHomeArchive("), source.indexOf("\nfunction verifyManagedHomeArchive("))
  const metadata = { schemaVersion: 1, scope: "state", id: "owned", sizeBytes: 33 * 1024 ** 3 + 1, sha256: "a".repeat(64) }
  let hashes = 0
  // IO is synthetic: exercise the actual broker policy and hash argv without
  // allocating, reading, or copying a multi-GiB archive.
  const brokerLifetime = new AbortController()
  const context = {
    brokerLifetime,
    managedHomeArchiveCoordinates: () => ({ candidate: "/private/archive", relative: ["states", "owned", "generation-abcdef", "home.tar.zst"] }),
    pinnedSharedPath: path => ({ path, fd: 7 }), dirname, join, homeArchiveMetadataMatches,
    readdirSync: () => ["home.tar.zst", "metadata.json"], readFileSync: () => JSON.stringify(metadata),
    fstatSync: () => ({ size: metadata.sizeBytes }), closeSync: () => {}, HOME_ARCHIVE_PROGRESS_TIMEOUT_MS,
    exactKeys: (value, keys) => assert.equal(Object.keys(value).sort().join(","), Array.from(keys).sort().join(",")),
    fail: message => { throw new Error(message) },
    digestPinnedHomeArchive: async (fd, timeout, signal) => {
      assert.equal(fd, 7)
      assert.equal(timeout, HOME_ARCHIVE_PROGRESS_TIMEOUT_MS)
      assert.equal(signal, brokerLifetime.signal)
      hashes++
      return metadata.sha256
    },
  }
  const inspect = runInNewContext("(" + body + ")", context)
  assert.equal((await inspect("/private/archive")).metadata.sizeBytes, metadata.sizeBytes)
  assert.equal(hashes, 1)
  metadata.sizeBytes = Number.MAX_SAFE_INTEGER + 1
  await assert.rejects(inspect("/private/archive"), /metadata is invalid/)
})

for (const eof of [false,true]) test(`default archive inactivity policy settles producer ${eof?'alive after EOF':'stalled after bytes'}`, async t => {
  const root=await mkdtemp(join(tmpdir(),'chariox-default-stall-'));t.after(()=>rm(root,{recursive:true,force:true}))
  const marker=join(root,'pid'),archive=join(root,'partial'),prior=join(root,'prior')
  await writeFile(prior,'known-good',{mode:0o600})
  const realTimeout=globalThis.setTimeout
  const pause=ms=>new Promise(resolve=>realTimeout(resolve,ms))
  t.mock.timers.enable({apis:['setTimeout']})
  let settled=false,result
  const capture=capturePrivateHomeArchive({command:process.execPath,args:['-e',`const fs=require('node:fs');fs.writeFileSync(${JSON.stringify(marker)},String(process.pid));process.stdout.write('private-partial',()=>{${eof?'fs.closeSync(1);':''}setInterval(()=>{},1000)})`],env:{PATH:process.env.PATH},destination:archive,minimumFreeBytes:0})
  const observed=capture.then(value=>{settled=true;result={value}},error=>{settled=true;result={error}})
  settleMockedCaptureAfterTest(t, observed, () => settled, pause, marker)
  for(let i=0;i<100;i++){try{await stat(marker);await stat(archive);if((await stat(archive)).size)break}catch{}await pause(10)}
  const pid=Number(await readFile(marker,'utf8'))
  t.mock.timers.tick(300001);await pause(50)
  const settledAtDeadline=settled
  if(!settled){process.kill(pid,'SIGKILL');await observed}
  else await observed
  assert.equal(await readFile(prior,'utf8'),'known-good')
  await assert.rejects(stat(archive),{code:'ENOENT'})
  assert.throws(()=>process.kill(pid,0),{code:'ESRCH'})
  assert.equal(settledAtDeadline,true,'default progress inactivity deadline must settle a stalled producer')
  assert.match(result.error.message,/made no progress/)
  const recovered=join(root,'recovered')
  const receipt=await capturePrivateHomeArchive({command:process.execPath,args:['-e',"process.stdout.write('recovered')"],env:{PATH:process.env.PATH},destination:recovered,minimumFreeBytes:0})
  assert.equal(receipt.sizeBytes,9)
  assert.equal(await readFile(recovered,'utf8'),'recovered')
})

for (const mode of ["startup", "descendant"]) {
  test("default inactivity settles " + mode + " with owned group cleanup", async t => {
    const { root, destination } = await fixture(t)
    const marker = join(root, "pid")
    const prior = join(root, "prior")
    await writeFile(prior, "known-good", { mode: 0o600 })
    const realTimeout = globalThis.setTimeout
    const pause = ms => new Promise(resolve => realTimeout(resolve, ms))
    t.mock.timers.enable({ apis: ["setTimeout"] })
    let body = "setInterval(()=>{},1000)"
    if (mode === "descendant") body = "const child=require('node:child_process').spawn(process.execPath,['-e',\"setInterval(()=>{},1000)\"],{stdio:['ignore',process.stdout,'ignore']});fs.writeFileSync(" + JSON.stringify(join(root, "descendant-pid")) + ",String(child.pid));process.stdout.write('private-partial',()=>process.exit(0))"
    const script = "const fs=require('node:fs');fs.writeFileSync(" + JSON.stringify(marker) + ",String(process.pid));" + body
    let settled = false
    const capture = capturePrivateHomeArchive({ command: process.execPath, args: ["-e", script],
      env: { PATH: process.env.PATH }, destination, minimumFreeBytes: 0 })
    const observed = capture.then(value => { settled = true; return { value } },
      error => { settled = true; return { error } })
    settleMockedCaptureAfterTest(t, observed, () => settled, pause, marker)
    for (let i = 0; i < 100; i++) {
      try { if (mode === "startup" ? await stat(marker) : (await stat(destination)).size > 0) break } catch {}
      await pause(10)
    }
    const pid = Number(await readFile(marker, "utf8"))
    t.mock.timers.tick(300_001)
    await pause(50)
    const settledAtDeadline = settled
    if (!settled) { try { process.kill(-pid, "SIGKILL") } catch {} }
    const result = await observed
    assert.equal(await readFile(prior, "utf8"), "known-good")
    await assert.rejects(stat(destination), { code: "ENOENT" })
    assert.throws(() => process.kill(pid, 0), { code: "ESRCH" })
    if (mode === "descendant") {
      const child = Number(await readFile(join(root, "descendant-pid"), "utf8"))
      // The host init reaps a child whose launcher exited before our watchdog.
      for (let i = 0; i < 100; i++) {
        try { process.kill(child, 0); await pause(10) } catch { break }
      }
      assert.throws(() => process.kill(child, 0), { code: "ESRCH" })
    }
    assert.equal(settledAtDeadline, true, "default inactivity must settle its producer")
    assert.match(result.error.message, /made no progress/)
    const recovered = join(root, "recovered")
    const receipt = await capturePrivateHomeArchive({ command: process.execPath,
      args: ["-e", "process.stdout.write('recovered')"], env: { PATH: process.env.PATH },
      destination: recovered, minimumFreeBytes: 0 })
    assert.equal(receipt.sizeBytes, 9)
  })
}

test("failed capture preserves a concurrently replaced archive name", async t => {
  const { root, destination, run } = await fixture(t)
  const moved = join(root, "our-partial")
  // MP-07/MP-11: advance inactivity only after actual producer bytes and the
  // replacement are observed; scheduler load must not race the 150ms deadline.
  const realTimeout = setTimeout
  const pause = ms => new Promise(resolve => realTimeout(resolve, ms))
  t.mock.timers.enable({ apis: ["setTimeout"] })
  const capture = run("process.stdout.write('private');setInterval(()=>{},1000)", { progressTimeoutMs: 150 })
  const observed = capture.catch(error => error)
  t.after(async () => { t.mock.timers.tick(151); await observed })
  const deadline = performance.now() + 5000
  while ((await stat(destination)).size === 0 && performance.now() < deadline) await pause(5)
  assert.equal(await readFile(destination, "utf8"), "private", "producer must acknowledge its partial archive")
  const { rename } = await import("node:fs/promises")
  await rename(destination, moved)
  await writeFile(destination, "replacement", { mode: 0o600 })
  t.mock.timers.tick(151)
  assert.match((await observed).message, /made no progress/)
  assert.equal(await readFile(destination, "utf8"), "replacement")
  assert.equal(await readFile(moved, "utf8"), "private")
})

test("closed successful, failed, and empty producers are never signaled after reaping", async t => {
  const original = process.kill
  let signals = 0
  t.mock.method(process, "kill", function(pid, signal) {
    if (pid < 0) signals++
    return original.call(process, pid, signal)
  })
  for (const script of ["process.stdout.write('complete')", "process.exit(7)", ""]) {
    const { run } = await fixture(t)
    await run(script).catch(() => {})
  }
  assert.equal(signals, 0)
})
