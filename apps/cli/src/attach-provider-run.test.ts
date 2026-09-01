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
