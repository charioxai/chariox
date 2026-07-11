import assert from "node:assert/strict"
import { access, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  assertHetznerBinaryFreshness,
  assertMatchingHetznerCheckoutCommit,
  ensureExecutionDirectory,
  hetznerNativeRuntimeCleanupCommand,
  hetznerWorktreeCleanupCommand,
  removeExecutionFile,
  waitForExecutionFileContent,
} from "./native-tui-remote-execution.mjs"

const LOCAL_COMMIT = "1111111111111111111111111111111111111111"
const REMOTE_COMMIT = "2222222222222222222222222222222222222222"

test("accepts a Hetzner checkout at the home commit", () => {
  assert.doesNotThrow(() => assertMatchingHetznerCheckoutCommit({
    localCommit: `${LOCAL_COMMIT}\n`,
    remoteCommit: LOCAL_COMMIT,
    remoteRepo: "/tmp/arroba-run",
  }))
})

test("rejects a stale Hetzner checkout before running a drill", () => {
  assert.throws(
    () => assertMatchingHetznerCheckoutCommit({
      localCommit: LOCAL_COMMIT,
      remoteCommit: REMOTE_COMMIT,
      remoteRepo: "/tmp/arroba-run",
    }),
    /remote worker checkout `\/tmp\/arroba-run` is at commit 2222.*home checkout expects 1111/,
  )
})

test("rejects unverifiable checkout revisions", () => {
  assert.throws(
    () => assertMatchingHetznerCheckoutCommit({
      localCommit: "not-a-commit",
      remoteCommit: REMOTE_COMMIT,
      remoteRepo: "/tmp/arroba-run",
    }),
    /could not verify local and remote checkout commits/,
  )
})

test("accepts Hetzner binaries newer than their tracked inputs", () => {
  assert.doesNotThrow(() => assertHetznerBinaryFreshness({
    remoteRepo: "/tmp/arroba-run",
    kernelNewerPath: "",
    relayNewerPath: "",
  }))
})

test("rejects stale Hetzner component binaries with the newer input", () => {
  assert.throws(
    () => assertHetznerBinaryFreshness({
      remoteRepo: "/tmp/arroba-run",
      kernelNewerPath: "apps/kernel/src/app.rs",
      relayNewerPath: "apps/relay/src/server.rs",
    }),
    /kernel binary is older than apps\/kernel\/src\/app\.rs; relay binary is older than apps\/relay\/src\/server\.rs/,
  )
})

test("builds scoped Hetzner worktree cleanup", () => {
  assert.equal(
    hetznerWorktreeCleanupCommand("/tmp/arroba-run", "/remote/worktree/arroba"),
    "git -C '/tmp/arroba-run' worktree remove --force '/remote/worktree/arroba' 2>/dev/null || rm -rf -- '/remote/worktree/arroba'; git -C '/tmp/arroba-run' worktree prune",
  )
})

test("builds deduplicated cleanup for native TUI runtime roots", () => {
  assert.equal(
    hetznerNativeRuntimeCleanupCommand([
      "/tmp/arb-remote-native-tui-42",
      "/tmp/arb-remote-native-tui-42-123456789",
      "/tmp/arb-remote-native-tui-42",
    ]),
    "rm -rf -- '/tmp/arb-remote-native-tui-42' '/tmp/arb-remote-native-tui-42-123456789'",
  )
})

test("rejects cleanup paths outside a native TUI runtime root", () => {
  assert.throws(
    () => hetznerNativeRuntimeCleanupCommand(["/tmp"]),
    /refusing to remove unexpected Hetzner native TUI runtime path/,
  )
})

test("same-host execution artifacts stay on the local machine", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-native-execution-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const outputDir = path.join(root, "nested", "outputs")
  const outputFile = path.join(outputDir, "permission.txt")

  await ensureExecutionDirectory({}, false, outputDir)
  await access(outputDir)
  await writeFile(outputFile, "permission-approved\n", "utf8")

  assert.equal(
    await waitForExecutionFileContent({}, false, outputFile, "permission-approved", 10),
    "permission-approved\n",
  )
  await removeExecutionFile({}, false, outputFile)
  assert.equal(await readFile(outputFile, "utf8").catch(() => null), null)
})
