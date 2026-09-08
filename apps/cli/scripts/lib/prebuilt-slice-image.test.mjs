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
test("missing and mismatched images fail before runtime startup", async (t) => {
  const cases = [
    { name: "missing image", records: [], error: /exactly one/ },
    { name: "ambiguous image", records: [record, record], error: /exactly one/ },
    { name: "missing immutable identity", records: [{}], error: /identity is missing/ },
    { name: "missing provenance", records: [{ ...record, Config: {} }], error: /exact current runtime source/ },
    { name: "stale source with matching protocol", records: [{ ...record, Config: { Labels: {
      ...record.Config.Labels, "io.chariox.runtime-source-revision": "revision-old",
    } } }], error: /exact current runtime source/ },
    { name: "matching source with stale protocol", records: [{ ...record, Config: { Labels: {
      ...record.Config.Labels, "io.chariox.relay-peer-protocol-version": "41",
    } } }], error: /relay protocol must match/ },
  ]
  for (const entry of cases) {
    await t.test(entry.name, async () => {
      await assert.rejects(validatePrebuiltSliceImage("image:tag", identity, async () => entry.records), entry.error)
    })
  }
})

test("Docker inspection failure propagates without selecting an image", async () => {
  const failure = new Error("Docker unavailable")
  await assert.rejects(validatePrebuiltSliceImage("image:tag", identity, async () => { throw failure }),
    error => error === failure)
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
