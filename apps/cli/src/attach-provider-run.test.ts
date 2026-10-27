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

// MP-08 / MP-10 / MP-11: reconnect preserves the kernel-owned account selection.
test("cold attach keeps the stored account instead of the CLI default", async () => {
  let selected = "";
  const result = await settleAttachProviderRun(session,
    { provider: "codex", model: "default", effort: "" }, "default", false, {
      launchProviderRun: async (_room, _provider, accountProfile) => {
        selected = accountProfile;
        return { id: "run-reconnected" } as never;
      },
      getSessionState: async () => session,
      tryGetProviderRun: async () => null,
    });
  assert.equal(result.action, "launched");
  assert.equal(selected, "codex-secondary");
});


for (const provider of ["claude", "claude-p", "claude-headless"]) {
  test(`${provider} attach reaches the TUI before a locked-Vault launch can wait for its UI`, async () => {
    const restored = { ...session, agents: [{ ...session.agents[0]!, provider, model: "claude-sonnet-5-5" }] }
    let launched = false
    const result = await settleAttachProviderRun(restored, { provider, model: "claude-sonnet-5-5", effort: "low" }, "product-linked", false, {
      getCredentialVaultStatus: async () => ({ unlocked: false }),
      launchProviderRun: async () => { launched = true; throw new Error("bootstrap must not wait for an unlock UI it has not rendered") },
      getSessionState: async () => restored,
      tryGetProviderRun: async () => null,
    })
    assert.equal(launched, false)
    assert.equal(result.action, "skipped")
    if (result.action !== "skipped") return
    assert.equal(result.reason, "credential_vault_locked")
    assert.equal(result.session, restored)
  })

}
// MP-08 / MP-10, PANE-13: attaching preserves the kernel-selected account.
test("attach launch keeps the existing agent account instead of CLI defaults", async () => {
  let account: string | null = null
  await settleAttachProviderRun(session, { provider: "codex", model: "default", effort: "" }, "default", false, {
    launchProviderRun: async (_session, _provider, profile) => { account = profile; return {} as never },
    getSessionState: async () => session,
    tryGetProviderRun: async () => null,
  })
  assert.equal(account, "codex-secondary")
})
