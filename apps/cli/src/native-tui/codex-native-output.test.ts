import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"
import { gzipSync } from "node:zlib"
import type { NativeProviderTransportGzipFragment } from "@chariox/kernel-client/kernel-types"
import { createCodexNativeOutputAssembler } from "./codex-native-output.js"

const pageBytes = 64 * 1024
const entropy = Buffer.concat(Array.from({ length: 4096 }, (_, i) => createHash("sha256").update(String(i)).digest()))
const original = JSON.stringify({ id: 1, result: { private: "native-canary🙂", data: entropy.toString("base64") } })
const compressed = gzipSync(original, { level: 1 })
const pages: NativeProviderTransportGzipFragment[] = []
for (let offset = 0; offset < compressed.length; offset += pageBytes) pages.push({
  offset, total_bytes: compressed.length, decoded_bytes: Buffer.byteLength(original),
  data: compressed.subarray(offset, offset + pageBytes).toString("base64"),
})
const reply = (gzip_fragment: NativeProviderTransportGzipFragment | null, closed = false) => ({
  connection_id: "owning", sequence: 1, frames: [], gzip_fragment, closed,
})

test("MP-08 / MP-10 / MP-11 bounded native pages emit the exact official frame once", () => {
  assert.ok(pages.length > 1)
  const assemble = createCodexNativeOutputAssembler()
  for (let i = 0; i < pages.length; i++) {
    assert.ok(Buffer.byteLength(JSON.stringify(reply(pages[i]!))) < 128 * 1024)
    assert.deepEqual(assemble(reply(pages[i]!, i === pages.length - 1)), i === pages.length - 1 ? [original] : [])
  }
})

test("MP-08 / MP-10 / MP-11 malformed native page bounds, ordering and encoding fail privately", () => {
  const bad = [
    { ...pages[0]!, offset: 1 },
    { ...pages[0]!, offset: Number.NaN },
    { ...pages[0]!, total_bytes: 17 * 1024 * 1024 },
    { ...pages[0]!, decoded_bytes: 16 * 1024 * 1024 + 1 },
    { ...pages[0]!, data: "native-canary%%%" },
    { ...pages[0]!, data: "A".repeat(90_000) },
  ]
  for (const fragment of bad) assert.throws(() => createCodexNativeOutputAssembler()(reply(fragment)), error => {
    assert.ok(error instanceof Error)
    assert.ok(!error.message.includes("canary"))
    return true
  })
  const assemble = createCodexNativeOutputAssembler()
  assemble(reply(pages[0]!))
  assert.throws(() => assemble(reply(pages[0]!)), /not contiguous/)
  assert.throws(() => createCodexNativeOutputAssembler()(reply(pages[0]!, true)), /before its last fragment/)
})

test("MP-11 native gzip expansion and invalid compressed data stay bounded", () => {
  const bomb = gzipSync("native-canary".repeat(100_000))
  assert.throws(() => createCodexNativeOutputAssembler()(reply({
    offset: 0, total_bytes: bomb.length, decoded_bytes: 16, data: bomb.toString("base64"),
  })), /invalid or exceeds its decoded bound/)
  assert.throws(() => createCodexNativeOutputAssembler()(reply({
    offset: 0, total_bytes: 4, decoded_bytes: 16, data: Buffer.from("nope").toString("base64"),
  })), /invalid or exceeds its decoded bound/)
})
