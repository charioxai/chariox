import assert from "node:assert/strict"
import test from "node:test"
import { readFile } from "node:fs/promises"
import { validatePrebuiltSliceImage } from "./prebuilt-slice-image.mjs"

const identity = { runtimeSourceRevision: "revision-a", protocolVersions: { relayPeer: 42 } }
const record = { Id: "sha256:image-a", Config: { Labels: {
  "io.chariox.runtime-source-revision": "revision-a",
  "io.chariox.relay-peer-protocol-version": "42",
} } }
test("prebuilt image validation is read-only and accepts exact source/protocol", async () => {
  assert.equal(await validatePrebuiltSliceImage(" image:tag ", identity, async image => {
    assert.equal(image, "image:tag")
    return [record]
  }), "sha256:image-a")
})
test("automatic image mode does not inspect a nonexistent prebuilt image", async () => {
  assert.equal(await validatePrebuiltSliceImage(undefined, identity, () => { throw Error("must not inspect") }), null)
})
test("missing and mismatched images fail before runtime startup", async () => {
  for (const records of [[], [{}], [{ ...record, Config: {} }], [{ ...record, Config: { Labels: {
    ...record.Config.Labels, "io.chariox.relay-peer-protocol-version": "41",
  } } }]]) {
    await assert.rejects(validatePrebuiltSliceImage("image:tag", identity, async () => records))
  }
})

test("room drill validates prebuilt identity before starting fixture or relay", async () => {
  const source = await readFile(new URL("../live-room-environment-pointer-click-drill.mjs", import.meta.url), "utf8")
  const check = source.indexOf("await validatePrebuiltSliceImage(")
  assert.ok(check > source.indexOf("async function run()"))
  assert.ok(check < source.indexOf("fixture = await startFixture()"))
  assert.ok(check < source.indexOf("const relay = spawn("))
  assert.ok(source.includes("slice image must contain the exact current runtime source"), "retain post-start validation")
  assert.match(source, /prebuiltSliceImageId = await validatePrebuiltSliceImage/)
  assert.match(source, /docker_image = \$\{JSON.stringify\(prebuiltSliceImageId\)\}/)
  assert.doesNotMatch(source, /JSON.stringify\(process.env.CHARIOX_ROOM_DRILL_IMAGE/)
})
