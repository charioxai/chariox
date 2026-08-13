import assert from "node:assert/strict"
import test from "node:test"

import {
  runtimeProviderRunForAgent,
  sameProviderRun,
  sessionActiveInteractionForAgent,
  sessionFocusedInteraction,
} from "./session-runtime-lookup.js"
import { makeAgent, makeSession } from "./shell-executor.test-support.js"
import type { RuntimeProviderRun } from "./kernel-types.js"

test("session active interaction lookup is scoped to an agent", () => {
  const session = makeSession({
    active_interactions: [{
      id: "interaction-1",
      agent_id: "agent-2",
      kind: "permission",
      level: "info",
      title: "Approve?",
      message: "Approve?",
      choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }],
      requested_at_ms: 1,
    }],
  })

  assert.equal(sessionActiveInteractionForAgent(session, "agent-2")?.id, "interaction-1")
  assert.equal(sessionActiveInteractionForAgent(session, "agent-1"), null)
  assert.equal(sessionActiveInteractionForAgent(session, null), null)
})

test("session focused interaction lookup follows focused agent fallback", () => {
  const session = makeSession({
    focused_agent_id: "agent-2",
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2", agent_ref: "agent-2" })],
    active_interactions: [{
      id: "interaction-2",
      agent_id: "agent-2",
      kind: "permission",
      level: "info",
      title: "Approve?",
      message: "Approve?",
      choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }],
      requested_at_ms: 1,
    }],
  })

  assert.equal(sessionFocusedInteraction(session)?.id, "interaction-2")
  assert.equal(sessionFocusedInteraction({ ...session, focused_agent_id: "missing-agent" }), null)
  assert.equal(sessionFocusedInteraction({ ...session, focused_agent_id: null }), null)
})

test("runtime provider run lookup requires matching agent ownership", () => {
  const run: RuntimeProviderRun = {
    id: "run-1",
    session_id: "session-1",
    provider: "codex",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    account_profile: "default",
    model: "gpt-5.2",
    variant: null,
    usage_tokens_total: null,
    state: "running",
  }

  assert.equal(runtimeProviderRunForAgent(run, "agent-1")?.id, "run-1")
  assert.equal(runtimeProviderRunForAgent(run, "agent-2"), null)
  assert.equal(runtimeProviderRunForAgent(null, "agent-1"), null)
})

test("same provider run compares identity and projected runtime metadata", () => {
  const run: RuntimeProviderRun = {
    id: "run-1",
    session_id: "session-1",
    provider: "codex",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    account_profile: "default",
    model: "gpt-5.2",
    variant: null,
    usage_tokens_total: 10,
    state: "running",
    client_interface: "native_tui",
    endpoint_mode: "managed",
    process_label: "codex-native",
    structured_endpoint: "endpoint-1",
    provider_session_id: "provider-session-1",
    working_directory: "/repo",
    started_at_ms: 1_000,
    last_activity_at_ms: 2_000,
    usage: {
      total_tokens: 10,
      last_tokens: 2,
      context_tokens: 8,
      context_window: 128_000,
    },
    control_capabilities: [{
      operation: "cancel",
      mode: "supported",
    }],
    external_provider_import: {
      external_provider_session_id: "external-session-1",
      external_provider: "codex",
      external_provider_session_provider_id: "provider-thread-1",
      observed_cursor: {
        last_observed_turn_id: "turn-1",
        last_observed_at_ms: 1_500,
        last_observed_merge_key: "merge-1",
        chariox_owned_observed_prompt_turn_ids: ["user-owned-1"],
      },
      last_observed_turn_id: "turn-1",
      last_observed_at_ms: 1_500,
      imported_at_ms: 900,
    },
  }

  assert.equal(sameProviderRun(run, { ...run }), true)
  assert.equal(sameProviderRun(run, { ...run, usage_tokens_total: 11 }), false)
  assert.equal(sameProviderRun(run, { ...run, usage: { ...run.usage, total_tokens: 11 } }), false)
  assert.equal(sameProviderRun(run, { ...run, state: "completed" }), false)
  assert.equal(sameProviderRun(run, { ...run, client_interface: "headless" }), false)
  assert.equal(sameProviderRun(run, { ...run, endpoint_mode: "direct" }), false)
  assert.equal(sameProviderRun(run, { ...run, process_label: "codex-headless" }), false)
  assert.equal(sameProviderRun(run, { ...run, structured_endpoint: "endpoint-2" }), false)
  assert.equal(sameProviderRun(run, { ...run, provider_session_id: "provider-session-2" }), false)
  assert.equal(sameProviderRun(run, { ...run, working_directory: "/other" }), false)
  assert.equal(sameProviderRun(run, { ...run, started_at_ms: 1_001 }), false)
  assert.equal(sameProviderRun(run, { ...run, last_activity_at_ms: 2_001 }), false)
  assert.equal(sameProviderRun(run, {
    ...run,
    control_capabilities: [{
      operation: "interrupt",
      mode: "supported",
    }],
  }), false)
  assert.equal(sameProviderRun(run, {
    ...run,
    external_provider_import: {
      ...run.external_provider_import!,
      observed_cursor: {
        ...run.external_provider_import!.observed_cursor,
        last_observed_turn_id: "turn-2",
      },
    },
  }), false)
  assert.equal(sameProviderRun(run, {
    ...run,
    external_provider_import: {
      ...run.external_provider_import!,
      observed_cursor: {
        ...run.external_provider_import!.observed_cursor,
        chariox_owned_observed_prompt_turn_ids: ["user-owned-2"],
      },
    },
  }), false)
})
