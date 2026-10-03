import test from "node:test"
import assert from "node:assert/strict"
import {verifyLocalHomeInventory, requireSafeLocalHomeVolume} from "../apps/kernel/slice-linux-docker/protected-local-home-scan.mjs"
import {requireRestoreVolumeReserve} from "../apps/kernel/slice-linux-docker/protected-home-restore.mjs"

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
  for (const result of [{status:0,stdout:"1 4096"},{status:1,stdout:"3000000 4096"},{status:0,stdout:"bad"},{status:0,stdout:"3000000 0"}]) assert.throws(()=>requireRestoreVolumeReserve(()=>result,"owned-restore"))
})
