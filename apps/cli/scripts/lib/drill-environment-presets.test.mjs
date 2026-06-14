import assert from "node:assert/strict"
import test from "node:test"

import {
  appendHetznerPassthrough,
  hetznerPassthroughMetadata,
  parseHetznerPassthroughArg,
} from "./drill-environment-presets.mjs"

test("parses Hetzner passthrough flags", () => {
  assert.deepEqual(parseHetznerPassthroughArg(["--hetzner-host", "root@example"], 0), {
    args: ["--hetzner-host", "root@example"],
    nextIndex: 1,
  })
  assert.deepEqual(parseHetznerPassthroughArg(["--hetzner-repo=/tmp/arroba"], 0), {
    args: ["--hetzner-repo", "/tmp/arroba"],
    nextIndex: 0,
  })
  assert.equal(parseHetznerPassthroughArg(["--include-hetzner"], 0), null)
  assert.throws(() => parseHetznerPassthroughArg(["--hetzner-key"], 0), /requires a value/)
})

test("appends Hetzner passthrough only for Hetzner scenarios", () => {
  const passthrough = ["--hetzner-host", "root@example"]

  assert.deepEqual(
    appendHetznerPassthrough(["drill.mjs"], { requires: ["hetzner"] }, passthrough),
    ["drill.mjs", "--hetzner-host", "root@example"],
  )
  assert.deepEqual(
    appendHetznerPassthrough(["drill.mjs"], { requires: ["remote"] }, passthrough),
    ["drill.mjs"],
  )
})

test("summarizes Hetzner passthrough without leaking values", () => {
  assert.deepEqual(
    hetznerPassthroughMetadata(["--hetzner-host", "root@example", "--hetzner-key", "/tmp/key"]),
    {
      hasHetznerHostOverride: true,
      hasHetznerSshIdentityOverride: true,
      hasHetznerRepoOverride: false,
    },
  )
})
