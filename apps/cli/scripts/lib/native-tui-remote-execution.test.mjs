import assert from "node:assert/strict"
import { access, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  applyClaudeWorkspaceTrust,
  assertHetznerBinaryFreshness,
  assertMatchingHetznerCheckoutCommit,
  ensureExecutionDirectory,
  hetznerNativeRuntimeCleanupCommand,
  hetznerNativeRuntimeTempDir,
  hetznerOpenCodeRuntimeProfileSeedCommand,
  hetznerWorktreeCleanupCommand,
  prepareClaudeWorkspaceTrustConfigText,
  removeExecutionFile,
  restoreClaudeConfigText,
  restoreClaudeWorkspaceTrust,
  seedLocalOpenCodeRuntimeProfile,
  stopHetznerRuntimeBeforeClaudeTrustRestore,
  waitForExecutionFileContent,
} from "./native-tui-remote-execution.mjs"

const LOCAL_COMMIT = "1111111111111111111111111111111111111111"
const REMOTE_COMMIT = "2222222222222222222222222222222222222222"

test("accepts a Hetzner checkout at the home commit", () => {
  assert.doesNotThrow(() => assertMatchingHetznerCheckoutCommit({
    localCommit: `${LOCAL_COMMIT}\n`,
    remoteCommit: LOCAL_COMMIT,
    remoteRepo: "/tmp/chariox-run",
  }))
})

test("rejects a stale Hetzner checkout before running a drill", () => {
  assert.throws(
    () => assertMatchingHetznerCheckoutCommit({
      localCommit: LOCAL_COMMIT,
      remoteCommit: REMOTE_COMMIT,
      remoteRepo: "/tmp/chariox-run",
    }),
    /remote worker checkout `\/tmp\/chariox-run` is at commit 2222.*home checkout expects 1111/,
  )
})

test("rejects unverifiable checkout revisions", () => {
  assert.throws(
    () => assertMatchingHetznerCheckoutCommit({
      localCommit: "not-a-commit",
      remoteCommit: REMOTE_COMMIT,
      remoteRepo: "/tmp/chariox-run",
    }),
    /could not verify local and remote checkout commits/,
  )
})

test("accepts Hetzner binaries newer than their tracked inputs", () => {
  assert.doesNotThrow(() => assertHetznerBinaryFreshness({
    remoteRepo: "/tmp/chariox-run",
    kernelNewerPath: "",
    relayNewerPath: "",
  }))
})

test("rejects stale Hetzner component binaries with the newer input", () => {
  assert.throws(
    () => assertHetznerBinaryFreshness({
      remoteRepo: "/tmp/chariox-run",
      kernelNewerPath: "apps/kernel/src/app.rs",
      relayNewerPath: "apps/relay/src/server.rs",
    }),
    /kernel binary is older than apps\/kernel\/src\/app\.rs; relay binary is older than apps\/relay\/src\/server\.rs/,
  )
})

test("builds scoped Hetzner worktree cleanup", () => {
  assert.equal(
    hetznerWorktreeCleanupCommand("/tmp/chariox-run", "/remote/worktree/chariox"),
    "git -C '/tmp/chariox-run' worktree remove --force '/remote/worktree/chariox' 2>/dev/null || rm -rf -- '/remote/worktree/chariox'; git -C '/tmp/chariox-run' worktree prune",
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

test("scopes the Hetzner worker temp directory to its removable runtime root", () => {
  assert.equal(
    hetznerNativeRuntimeTempDir("/tmp/arb-remote-native-tui-42-123456789"),
    "/tmp/arb-remote-native-tui-42-123456789/tmp",
  )
  assert.throws(
    () => hetznerNativeRuntimeTempDir("/tmp"),
    /refusing unexpected Hetzner native TUI runtime root/,
  )
})

test("seeds only OpenCode credentials and model catalog into an isolated runtime", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-opencode-profile-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const sourceDataHome = path.join(root, "source-data")
  const sourceCacheHome = path.join(root, "source-cache")
  const destinationXdgDataHome = path.join(root, "runtime-data")
  const destinationXdgCacheHome = path.join(root, "runtime-cache")
  await mkdir(sourceDataHome, { recursive: true })
  await mkdir(sourceCacheHome, { recursive: true })
  await writeFile(path.join(sourceDataHome, "auth.json"), "credential\n")
  await writeFile(path.join(sourceDataHome, "opencode.db"), "session-state\n")
  await writeFile(path.join(sourceCacheHome, "models.json"), "catalog\n")
  await writeFile(path.join(sourceCacheHome, "version"), "1\n")

  const credentialPath = await seedLocalOpenCodeRuntimeProfile({
    sourceDataHome,
    sourceCacheHome,
    destinationXdgDataHome,
    destinationXdgCacheHome,
  })

  assert.equal(await readFile(credentialPath, "utf8"), "credential\n")
  assert.equal((await stat(credentialPath)).mode & 0o777, 0o600)
  assert.equal(
    await readFile(path.join(destinationXdgCacheHome, "opencode", "models.json"), "utf8"),
    "catalog\n",
  )
  const versionPath = path.join(destinationXdgCacheHome, "opencode", "version")
  assert.equal(await readFile(versionPath, "utf8"), "1\n")
  assert.equal((await stat(versionPath)).mode & 0o777, 0o600)
  await assert.rejects(
    readFile(path.join(destinationXdgDataHome, "opencode", "opencode.db")),
    /ENOENT/,
  )
})

test("removes a copied OpenCode credential when catalog seeding fails", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-opencode-profile-failure-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const sourceDataHome = path.join(root, "source-data")
  const sourceCacheHome = path.join(root, "source-cache")
  const destinationXdgDataHome = path.join(root, "runtime-data")
  await mkdir(sourceDataHome, { recursive: true })
  await mkdir(path.join(sourceCacheHome, "models.json"), { recursive: true })
  await writeFile(path.join(sourceDataHome, "auth.json"), "credential\n")
  await writeFile(path.join(sourceCacheHome, "version"), "1\n")

  await assert.rejects(seedLocalOpenCodeRuntimeProfile({
    sourceDataHome,
    sourceCacheHome,
    destinationXdgDataHome,
    destinationXdgCacheHome: path.join(root, "runtime-cache"),
  }))
  await assert.rejects(
    readFile(path.join(destinationXdgDataHome, "opencode", "auth.json")),
    /ENOENT/,
  )
})

test("does not seed a partial OpenCode model catalog", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-opencode-partial-catalog-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const sourceDataHome = path.join(root, "source-data")
  const sourceCacheHome = path.join(root, "source-cache")
  const destinationXdgCacheHome = path.join(root, "runtime-cache")
  await mkdir(sourceDataHome, { recursive: true })
  await mkdir(sourceCacheHome, { recursive: true })
  await writeFile(path.join(sourceCacheHome, "models.json"), "catalog\n")

  await seedLocalOpenCodeRuntimeProfile({
    sourceDataHome,
    sourceCacheHome,
    destinationXdgDataHome: path.join(root, "runtime-data"),
    destinationXdgCacheHome,
  })

  await assert.rejects(
    readFile(path.join(destinationXdgCacheHome, "opencode", "models.json")),
    /ENOENT/,
  )
})

test("seeds a scoped Hetzner OpenCode runtime without its session database", () => {
  const command = hetznerOpenCodeRuntimeProfileSeedCommand(
    "/tmp/arb-remote-native-tui-42-123456789",
  )
  assert.match(command, /\/root\/\.local\/share\/opencode\/auth\.json/)
  assert.match(command, /\/root\/\.cache\/opencode\/models\.json/)
  assert.match(command, /\/root\/\.cache\/opencode\/version/)
  assert.doesNotMatch(command, /opencode\.db/)
  assert.throws(
    () => hetznerOpenCodeRuntimeProfileSeedCommand("/tmp"),
    /refusing unexpected Hetzner native TUI runtime root/,
  )
})

test("stops the Hetzner runtime before restoring Claude trust", async () => {
  const calls = []

  await stopHetznerRuntimeBeforeClaudeTrustRestore({
    stopWorker: async () => calls.push("worker-stopped"),
    stopRelay: async () => calls.push("relay-stopped"),
    restoreTrust: async () => calls.push("trust-restored"),
  })

  assert.deepEqual(calls, ["worker-stopped", "relay-stopped", "trust-restored"])
})

test("Claude workspace trust can be applied and restored without changing sibling projects", () => {
  const original = {
    theme: "dark",
    projects: {
      "/existing": {
        allowedTools: ["Read"],
        hasTrustDialogAccepted: true,
        projectOnboardingSeenCount: 4,
      },
    },
  }
  const prepared = applyClaudeWorkspaceTrust(original, "/remote/worktree")
  assert.deepEqual(prepared.config.projects["/remote/worktree"], {
    allowedTools: ["Read"],
    hasTrustDialogAccepted: true,
    projectOnboardingSeenCount: 4,
  })
  assert.equal(original.projects["/remote/worktree"], undefined)
  assert.deepEqual(
    restoreClaudeWorkspaceTrust(prepared.config, prepared.state),
    original,
  )
})

test("Claude workspace trust restores an existing per-worktree entry exactly", () => {
  const original = {
    projects: {
      "/remote/worktree": {
        allowedTools: [],
        hasTrustDialogAccepted: false,
        projectOnboardingSeenCount: 0,
        custom: "preserve-me",
      },
    },
  }
  const prepared = applyClaudeWorkspaceTrust(original, "/remote/worktree")
  assert.equal(prepared.config.projects["/remote/worktree"].hasTrustDialogAccepted, true)
  assert.deepEqual(
    restoreClaudeWorkspaceTrust(prepared.config, prepared.state),
    original,
  )
})

test("Claude workspace trust restores the original config bytes after global mutations", () => {
  const originalText = `${JSON.stringify({
    numStartups: 7,
    projects: {
      "/existing": {
        hasTrustDialogAccepted: true,
        projectOnboardingSeenCount: 2,
      },
    },
  }, null, 4)}\n`
  const prepared = prepareClaudeWorkspaceTrustConfigText(originalText, "/remote/worktree")
  prepared.config.numStartups = 99
  prepared.config.projects["/existing"].projectOnboardingSeenCount = 8

  assert.equal(restoreClaudeConfigText(prepared.state), originalText)
})

test("Claude workspace trust removes config created during a drill when none existed", () => {
  const prepared = prepareClaudeWorkspaceTrustConfigText(null, "/remote/worktree")

  assert.equal(restoreClaudeConfigText(prepared.state), null)
})

test("same-host execution artifacts stay on the local machine", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-native-execution-test-"))
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
