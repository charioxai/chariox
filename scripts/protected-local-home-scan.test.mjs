import test from "node:test"
import {spawn, spawnSync} from "node:child_process"
import {closeSync, mkdtempSync, openSync, rmSync, writeFileSync} from "node:fs"
import {tmpdir} from "node:os"
import {join} from "node:path"
import {fileURLToPath} from "node:url"
import assert from "node:assert/strict"
import {verifyLocalHomeInventory, requireSafeLocalHomeVolume} from "../apps/kernel/slice-linux-docker/protected-local-home-scan.mjs"
import {RestoreRefusal, extractCompressedArchive, requireRestoreVolumeReserve, validateCompressedArchive, verifyFreshRestoreTarget} from "../apps/kernel/slice-linux-docker/protected-home-restore.mjs"

test("local inventory defers only transient metadata until quiescence", () => {
  const transient = Buffer.from(".config/chromium/SingletonSocket\0l\0/tmp/chrome-socket\0" + "700\0")
  assert.doesNotThrow(() => verifyLocalHomeInventory(transient, {quiesced:false}))
  assert.throws(() => verifyLocalHomeInventory(transient))
  assert.throws(() => verifyLocalHomeInventory(Buffer.from(".config/file\0f\0\0" + "6755\0"), {quiesced:false}))
  assert.throws(() => verifyLocalHomeInventory(Buffer.from("file\0f\0\0" + "600\0"), {maxEntries:0}))
})
test("local scanner uses only selected image and exact read-only volume, and cleans its own helper on refusal", () => {
  const calls=[]
  const volume="chariox-slice-test-home"
  const docker=args => {
    calls.push(args)
    if (args[0]==="volume") return {status:0,stdout:JSON.stringify([{Name:volume,Driver:"local",Scope:"local",Options:null}])}
    if (args[0]==="run") return {status:0,stdout:Buffer.from("file\0f\0\0" + "600\0")}
    return {status:0,stdout:""}
  }
  requireSafeLocalHomeVolume({volume,docker,enrollment:{ownerUid:1234,workerImageId:`sha256:${"a".repeat(64)}`}})
  const run=calls.find(args=>args[0]==="run")
  assert.ok(run.includes(`type=volume,src=${volume},dst=/home-src,readonly`))
  assert.ok(run.includes(`sha256:${"a".repeat(64)}`))
  assert.ok(!run.join(" ").includes("/var/lib/docker"))
  assert.deepEqual(calls.at(-1),["rm","-f",run[run.indexOf("--name")+1]])
  assert.throws(()=>requireSafeLocalHomeVolume({volume,docker:args=>args[0]==="volume"?{status:0,stdout:JSON.stringify([{Name:volume,Driver:"local",Scope:"local",Options:{device:"/etc"}}])}:docker(args),enrollment:{ownerUid:1234}}))
})
test("restore admission uses actual selected volume filesystem and fails closed on bad or low results", () => {
  const calls=[]
  requireRestoreVolumeReserve(args=>{calls.push(args);return {status:0,stdout:"3000000 4096\n"}},"owned-restore")
  assert.deepEqual(calls[0],["exec","-u","root","owned-restore","/usr/bin/stat","-f","-c","%a %S","/home-dst"])
  for (const [result, reason] of [
    [{status:0,stdout:"1 4096"}, /restore volume has less than 10240 MiB free/],
    [{status:1,stdout:"3000000 4096"}, /restore volume free space is unreadable/],
    [{status:0,stdout:"bad"}, /restore volume free space is unreadable/],
    [{status:0,stdout:"3000000 0"}, /restore volume free space is unreadable/],
  ]) assert.throws(()=>requireRestoreVolumeReserve(()=>result,"owned-restore"), reason)
})

test("a refused protected home restore names the failed check without private paths", () => {
  const script = fileURLToPath(new URL("../apps/kernel/slice-linux-docker/protected-home-restore.mjs", import.meta.url))
  const refused = spawnSync(process.execPath, [script, "chariox-slice-x-home-restore-1", "chariox-slice-x-home-gbad", "/etc/passwd"],
    {env: {PATH: "/usr/bin:/bin", CHARIOX_SLICE_NAME: "chariox-slice-x", CHARIOX_SLICE_RESTORE_GENERATION: "a".repeat(32)}, encoding: "utf8", timeout: 30_000})
  assert.equal(refused.status, 1)
  assert.equal(refused.stderr.trim(),
    "Saved slice home restore was refused (restore arguments are invalid); existing identity and saved state are preserved")
  assert.throws(() => verifyFreshRestoreTarget({Config: {Labels: {}}}, "helper", "volume", Buffer.alloc(0)),
    (error) => error instanceof RestoreRefusal && error.reason === "restore target is not a fresh, empty, owned home volume")
})

test("a failing restore child is reported by its own exit, not by the broken pipe into it", async () => {
  const root = mkdtempSync(join(tmpdir(), "chariox-restore-reason-"))
  const archive = join(root, "home.tar.zst")
  writeFileSync(archive, Buffer.alloc(8 * 1024 * 1024, 7))
  // Docker stands in as `cat`; the validator or extraction exits at once without
  // reading, so the stream into it breaks while it is exiting.
  const exits = (status) => (command, args, options) => command === "/usr/bin/docker" && args.includes("zstd")
    ? spawn("cat", [], {stdio: options.stdio})
    : spawn("/bin/sh", ["-c", `exit ${status}`], {stdio: options.stdio})
  // A destroyed read stream may close the shared descriptor, so each attempt opens its own.
  const attempt = async (run) => {
    const fd = openSync(archive, "r")
    try { return await run(fd) } finally { try { closeSync(fd) } catch {} }
  }
  try {
    for (let round = 0; round < 5; round++) {
      await assert.rejects(attempt(fd => validateCompressedArchive(fd, "helper", {}, () => {}, exits(3))),
        (error) => error instanceof RestoreRefusal && error.reason === "archive validation exited with status 3")
      await assert.rejects(attempt(fd => extractCompressedArchive(fd, "helper", {}, () => {}, exits(2))),
        (error) => error instanceof RestoreRefusal && error.reason === "archive extraction exited with status 2")
    }
  } finally {
    rmSync(root, {recursive: true, force: true})
  }
})
