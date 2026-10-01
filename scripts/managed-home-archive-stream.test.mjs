import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { capturePrivateHomeArchive } from "../apps/kernel/slice-linux-docker/managed-home-archive-stream.mjs"

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
