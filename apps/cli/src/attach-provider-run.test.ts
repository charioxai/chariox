import assert from "node:assert/strict"
import test from "node:test"

import { settleAttachProviderRun } from "./attach-provider-run.js"
import { LocalIpcError } from "./ipc.js"
import type { RuntimeSession } from "./cli-types.js"

const session: RuntimeSession = {
  id: "session-vault",
  project_id: "project-default",
  alias: "vault",
  workspace_id: "/tmp/workspace",
  worktree_id: "/tmp/workspace",
  created_at_ms: 1,
  status: "Active",
  active_provider_run_id: null,
  attachment_ids: ["attachment-vault"],
  active_prompt: null,
  queued_prompts: [],
  focused_agent_id: "agent-vault",
  max_agents: 6,
  agents: [{
    id: "agent-vault",
    agent_ref: "agent-vault",
    session_id: "session-vault",
    alias: "agent-vault",
    provider: "codex",
    account_profile: "codex-secondary",
    model: "gpt-5.6-luna",
    effort: "low",
    worktree_id: "/tmp/workspace",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }],
  config_state: { version: 1, values: {} },
}

test("MP-08/MP-10 TUI bootstrap can render before a launch asks for human input", async () => {
  let launches = 0
  let release!: () => void
  const humanReply = new Promise<void>(resolve => { release = resolve })
  const task = settleAttachProviderRun(session,
    { provider: "claude-p", model: "claude-haiku-5-5", effort: "low" }, "work", false, {
      deferLaunch: true,
      launchProviderRun: async () => { launches++; await humanReply; return {} as never },
      getSessionState: async () => session,
      tryGetProviderRun: async () => null,
    })
  try {
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(launches, 0, "an unlock wait cannot start before the TUI mounts")
    const result = await task
    assert.equal(result.action, "deferred")
    if (result.action !== "deferred") return
    assert.equal(result.isPending(), true)
    const first = result.start()
    assert.equal(result.start(), first, "mounting twice cannot launch twice")
    assert.equal(launches, 1)
    release()
    await first
    assert.equal(result.isPending(), false)
  } finally {
    release()
    await task
  }
})

test("settleAttachProviderRun keeps the TUI attachable when the credential vault is locked", async () => {
  const result = await settleAttachProviderRun(
    session,
    { provider: "codex", model: "gpt-5.6-luna", effort: "low" },
    "codex-secondary",
    false,
    {
      launchProviderRun: async () => {
        throw new LocalIpcError(
          "handle kernel response",
          "local transport `credential_vault_locked` failed: Chariox vault is locked",
          "local_transport_error",
          true,
        )
      },
      getSessionState: async () => session,
      tryGetProviderRun: async () => null,
    },
  )

  assert.equal(result.action, "skipped")
  if (result.action !== "skipped") return
  assert.equal(result.reason, "credential_vault_locked")
  assert.equal(result.providerRun, null)
  assert.equal(result.targetAgent?.id, "agent-vault")
})

test("MP-08/MP-10 attaching to an existing unlock does not launch the agent twice", async () => {
  let launches = 0
  const result = await settleAttachProviderRun({ ...session, active_interactions: [{
    id: "vault-unlock-existing", agent_id: "agent-vault", kind: "choice", level: "critical",
    title: "Unlock Chariox Vault", message: "Enter the passphrase.", requested_at_ms: 1,
    choices: [{ id: "cancel", label: "Cancel", reply: "cancel" }],
  }] }, { provider: "claude-p", model: "claude-haiku-5-5", effort: "low" }, "work", false, {
    deferLaunch: true,
    launchProviderRun: async () => { launches++; return {} as never },
    getSessionState: async () => session,
    tryGetProviderRun: async () => null,
  })
  assert.equal(result.action, "skipped")
  assert.equal(launches, 0)
})

test("settleAttachProviderRun recognizes the structured vault-locked error code", async () => {
  const result = await settleAttachProviderRun(
    session,
    { provider: "codex", model: "gpt-5.6-luna", effort: "low" },
    "codex-secondary",
    false,
    {
      launchProviderRun: async () => {
        throw new LocalIpcError(
          "handle kernel response",
          "vault access is unavailable",
          "credential_vault_locked",
          false,
        )
      },
      getSessionState: async () => session,
      tryGetProviderRun: async () => null,
    },
  )

  assert.equal(result.action, "skipped")
  if (result.action !== "skipped") return
  assert.equal(result.reason, "credential_vault_locked")
})

test("settleAttachProviderRun still surfaces unrelated provider launch failures", async () => {
  const launchError = new LocalIpcError(
    "handle kernel response",
    "provider authentication failed",
    "local_transport_error",
    false,
  )

  await assert.rejects(
    settleAttachProviderRun(
      session,
      { provider: "codex", model: "gpt-5.6-luna", effort: "low" },
      "codex-secondary",
      false,
      {
        launchProviderRun: async () => { throw launchError },
        getSessionState: async () => session,
        tryGetProviderRun: async () => null,
      },
    ),
    (error) => error === launchError,
  )
})

test("MP-08 reattach relaunches an idle agent with its own account, not the CLI default", async () => {
  const launched: string[] = []
  const result = await settleAttachProviderRun(
    session,
    { provider: "codex", model: "gpt-5.6-luna", effort: "low" },
    "default",
    false,
    {
      launchProviderRun: async (_sessionId, _provider, accountProfile) => {
        launched.push(accountProfile)
        return { id: "provider-run-reattach" } as never
      },
      getSessionState: async () => session,
      tryGetProviderRun: async () => null,
    },
  )
  assert.equal(result.action, "launched")
  assert.deepEqual(launched, ["codex-secondary"])
})

test("attaching an existing agent preserves its explicit account instead of CLI defaults", async () => {
  let launchedAccount: string | null = null
  const launchError = new Error("stop before provider execution")
  await assert.rejects(settleAttachProviderRun(
    session,
    { provider: "opencode", model: "default", effort: "" },
    "default",
    false,
    {
      launchProviderRun: async (_id, _provider, account) => {
        launchedAccount = account
        throw launchError
      },
      getSessionState: async () => session,
      tryGetProviderRun: async () => null,
    },
  ), (error) => error === launchError)
  assert.equal(launchedAccount, "codex-secondary")
})
