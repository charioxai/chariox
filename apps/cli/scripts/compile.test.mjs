import assert from "node:assert/strict"
import { test } from "node:test"

import { parseCompileArgs, pinOpenTuiPlatform, targets } from "./compile.mjs"

test("the release targets are exactly the Phase 1 platforms", () => {
  assert.deepEqual(Object.keys(targets).sort(), ["darwin-arm64", "linux-x64"])
  assert.equal(targets["linux-x64"].bun, "bun-linux-x64-baseline")
  assert.equal(targets["darwin-arm64"].bun, "bun-darwin-arm64")
})

test("arguments name a known target, an output file and an optional semantic version", () => {
  assert.deepEqual(parseCompileArgs(["--target", "linux-x64", "--outfile", "out/chariox", "--version", "0.2.0-rc.1"]),
    { target: "linux-x64", outfile: "out/chariox", version: "0.2.0-rc.1" })
  assert.deepEqual(parseCompileArgs(["--outfile", "chariox", "--target", "darwin-arm64"]),
    { target: "darwin-arm64", outfile: "chariox", version: undefined })
  for (const [argv, message] of [
    [["--target", "windows-x64", "--outfile", "x"], /--target must be one of/],
    [["--target", "linux-x64"], /--outfile is required/],
    [["--target", "linux-x64", "--outfile"], /--outfile needs a value/],
    [["--target", "linux-x64", "--outfile", "--version"], /--outfile needs a value/],
    [["--target", "linux-x64", "--target", "linux-x64", "--outfile", "x"], /given twice/],
    [["--target", "linux-x64", "--outfile", "x", "--version", "v1"], /semantic version/],
    [["--target", "linux-x64", "--outfile", "x", "--minify", "1"], /unknown argument/],
  ]) {
    assert.throws(() => parseCompileArgs(argv), message)
  }
})

test("OpenTUI's run-time platform import is pinned to the target's native package", () => {
  const source = "var module = await import(`@opentui/core-${process.platform}-${process.arch}/index.ts`);\nexport { module };\n"
  const linux = pinOpenTuiPlatform(source, targets["linux-x64"])
  assert.equal(linux.pinned, 1)
  assert.match(linux.code, /await import\("@opentui\/core-linux-x64\/index\.ts"\)/)
  assert.doesNotMatch(linux.code, /process\.platform/)
  assert.match(pinOpenTuiPlatform(source, targets["darwin-arm64"]).code, /"@opentui\/core-darwin-arm64\/index\.ts"/)
  // Other OpenTUI modules pass through untouched.
  assert.deepEqual(pinOpenTuiPlatform("export const x = 1\n", targets["linux-x64"]), { code: "export const x = 1\n", pinned: 0 })
})
