import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, stat } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  cleanupSliceRuntime,
  failResultOnSliceCleanupErrors,
} from "./live-provider-thread-transfer-slice-scenarios.mjs"
import {
  providerStateCopySpecs,
  transferProviderStateToWorker,
} from "./live-provider-thread-transfer-provider-state.mjs"
import {
  cleanupSliceModeProviderCredentials,
  normalizeProviderOutputText,
  providerThreadSliceOptLevel,
  providersNeedClaudeCredentials,
  sliceProviderAuthImportRequest,
  terminalProviderHistoryError,
  workerResumeDaemonEnv,
  writeClaudeCredentialsPayload,
} from "./live-provider-thread-transfer-runtime.mjs"

test("provider output marker matching removes ANSI without accepting subsequences", () => {
  const expected = "THREAD_TRANSFER_READY"
  assert.equal(
    normalizeProviderOutputText("THREAD_\x1b[31mTRANSFER\x1b[0m_READY").includes(expected),
    true,
  )
  assert.equal(
    normalizeProviderOutputText("THREAD_TRANSFER was requested with suffix _READY").includes(expected),
    false,
  )
})

test("Claude provider aliases request isolated credentials", () => {
  assert.equal(providersNeedClaudeCredentials(["codex", "opencode"]), false)
  assert.equal(providersNeedClaudeCredentials(["claude"]), true)
  assert.equal(providersNeedClaudeCredentials(["claude-p"]), true)
  assert.equal(providersNeedClaudeCredentials(["claude-headless"]), true)
})

test("Claude provider state uses the selected provider home", () => {
  const specs = providerStateCopySpecs("claude-headless", { HOME: "/isolated/provider-home" })
  assert.deepEqual(
    specs.map((spec) => spec.source),
    ["/isolated/provider-home/.claude", "/isolated/provider-home/.claude.json"],
  )
})

test("provider state transfer copies into an isolated worker home", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-provider-state-test-"))
  try {
    const sourceHome = path.join(root, "source")
    const destinationHome = path.join(root, "destination")
    await writeClaudeCredentialsPayload(
      path.join(sourceHome, ".claude", "projects", "session.json"),
      Buffer.from('{"session":"one"}\n'),
    )
    await writeClaudeCredentialsPayload(
      path.join(sourceHome, ".claude.json"),
      Buffer.from('{"hasCompletedOnboarding":true}\n'),
    )

    const evidence = await transferProviderStateToWorker({
      provider: "claude-headless",
      sourceProviderEnv: { HOME: sourceHome },
      destinationProviderEnv: { HOME: destinationHome },
    })

    assert.equal(evidence.copied.length, 2)
    assert.equal(
      await readFile(path.join(destinationHome, ".claude", "projects", "session.json"), "utf8"),
      '{"session":"one"}\n',
    )
    assert.equal(
      await readFile(path.join(destinationHome, ".claude.json"), "utf8"),
      '{"hasCompletedOnboarding":true}\n',
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Claude credential payloads are validated and written mode 600", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-claude-credentials-test-"))
  try {
    const destination = path.join(root, ".claude", ".credentials.json")
    await writeClaudeCredentialsPayload(destination, Buffer.from('{"oauth":"test"}\n'))
    assert.equal(await readFile(destination, "utf8"), '{"oauth":"test"}\n')
    assert.equal((await stat(destination)).mode & 0o777, 0o600)
    await assert.rejects(
      writeClaudeCredentialsPayload(path.join(root, "invalid.json"), Buffer.from("[]")),
      /JSON object/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("slice provider credentials are removed without deleting provider state", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-slice-credentials-test-"))
  try {
    const codexHome = path.join(root, "codex")
    const opencodeHome = path.join(root, "opencode")
    const claudeSecretRoot = path.join(root, "claude-secrets")
    await writeClaudeCredentialsPayload(
      path.join(codexHome, "auth.json"),
      Buffer.from('{"token":"codex"}\n'),
    )
    await writeClaudeCredentialsPayload(
      path.join(opencodeHome, "auth.json"),
      Buffer.from('{"token":"opencode"}\n'),
    )
    await writeClaudeCredentialsPayload(
      path.join(claudeSecretRoot, "credentials.json"),
      Buffer.from('{"token":"claude"}\n'),
    )
    await writeClaudeCredentialsPayload(
      path.join(codexHome, "sessions", "state.json"),
      Buffer.from('{"session":"retained"}\n'),
    )

    await cleanupSliceModeProviderCredentials({
      CODEX_HOME: codexHome,
      OPENCODE_DATA_HOME: opencodeHome,
      CHARIOX_PROVIDER_THREAD_CODEX_AUTH_COPIED: "1",
      CHARIOX_PROVIDER_THREAD_OPENCODE_AUTH_COPIED: "1",
      CHARIOX_PROVIDER_THREAD_CLAUDE_SECRET_ROOT: claudeSecretRoot,
    })

    await assert.rejects(stat(path.join(codexHome, "auth.json")), { code: "ENOENT" })
    await assert.rejects(stat(path.join(opencodeHome, "auth.json")), { code: "ENOENT" })
    await assert.rejects(stat(claudeSecretRoot), { code: "ENOENT" })
    assert.equal(
      await readFile(path.join(codexHome, "sessions", "state.json"), "utf8"),
      '{"session":"retained"}\n',
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("slice restart cleanup resets saved state before deleting the slice", async () => {
  const requests = []
  const evidence = {}
  const client = {
    async send(request) {
      requests.push(request)
      if ("ResetSliceState" in request) throw new Error("saved state cleanup failed")
      return { SliceDeleted: { slice: { id: "slice-1" } } }
    },
  }

  await cleanupSliceRuntime(client, "slice-1", evidence, { resetSavedState: true })

  assert.deepEqual(requests, [
    { ResetSliceState: { slice_ref: "slice-1" } },
    { DeleteSlice: { slice_ref: "slice-1" } },
  ])
  assert.equal(evidence.slice_state_cleanup_error, "saved state cleanup failed")
  assert.equal(evidence.slice_cleanup_error, undefined)
})

test("slice cleanup errors fail an otherwise passing drill result", () => {
  const result = {
    status: "passed",
    evidence: { slice_state_cleanup_error: "state image remained" },
    errors: [],
  }

  failResultOnSliceCleanupErrors(result, { resetSavedState: true })

  assert.equal(result.status, "failed")
  assert.deepEqual(result.errors, ["slice cleanup failed: state image remained"])
})

test("provider thread transfer fails fast on terminal provider history", () => {
  const failure = terminalProviderHistoryError([
    { kind: "notice", text: "provider is starting" },
    { kind: "provider_error", text: "account balance exhausted" },
  ])

  assert.equal(failure?.text, "account balance exhausted")
  assert.equal(
    terminalProviderHistoryError([
      {
        kind: "notice",
        text: "Provider run `provider-run-1` for `claude-headless` ended unexpectedly. No active prompt was running.",
      },
    ])?.kind,
    "notice",
  )
})

test("provider thread transfer ignores nonterminal provider history", () => {
  assert.equal(terminalProviderHistoryError([
    { kind: "notice", text: "provider is starting" },
    { kind: "provider_output", text: "done" },
  ]), null)
})

test("worker resume daemons keep Chariox state inside the drill runtime root", () => {
  const env = workerResumeDaemonEnv({
    ports: { relayPort: 4000 },
    root: "/tmp/provider-runtime",
    relayToken: "token",
    daemonId: "worker-1",
    daemonAlias: "worker",
    machineId: "machine-1",
    machineAlias: "machine",
    acceptRemoteLeases: true,
    socketName: "worker.sock",
    kernelPort: 4001,
    mcpPort: 4002,
    openCodePort: 4003,
    codexPort: 4004,
    providerEnv: {},
  })

  assert.equal(env.CHARIOX_HOME, "/tmp/provider-runtime/worker-1-xdg-config/chariox")
  assert.equal(env.CHARIOX_SESSION_HISTORY_DIR, "/tmp/provider-runtime/worker-1-history")
  assert.equal(env.CHARIOX_DAEMON_SOCKET, "/tmp/provider-runtime/worker.sock")
})

test("slice provider auth import includes the required managed account profile", () => {
  assert.deepEqual(sliceProviderAuthImportRequest("slice-1", "codex"), {
    ImportSliceProviderAuth: {
      slice_ref: "slice-1",
      provider: "codex",
      account_profile: "default",
    },
  })
})

test("provider thread slice builds use a bounded optimization level", () => {
  assert.equal(providerThreadSliceOptLevel({}), "1")
  assert.equal(
    providerThreadSliceOptLevel({ CHARIOX_PROVIDER_THREAD_SLICE_OPT_LEVEL: "0" }),
    "0",
  )
  assert.throws(
    () => providerThreadSliceOptLevel({ CHARIOX_PROVIDER_THREAD_SLICE_OPT_LEVEL: "fast" }),
    /optimization level/,
  )
})
