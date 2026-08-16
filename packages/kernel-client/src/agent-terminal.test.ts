import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises"
import test from "node:test"
import { tmpdir } from "node:os"
import { join } from "node:path"

import { AgentTerminal, type AgentTerminalCatalog } from "./agent-terminal.js"
import type { KernelEvent } from "./kernel-events.js"

const catalog: AgentTerminalCatalog = {
  revision: "catalog-test",
  nodes: [
    {
      id: "session-list",
      label: "list",
      description: "List available sessions",
      value: "/session list",
      kind: "command",
      execution_target: "kernel",
      surfaces: ["session", "waiting_room"],
      search_aliases: ["active sessions"],
    },
    {
      id: "agent",
      label: "/agent",
      description: "Manage agents",
      value: "/agent ",
      kind: "group",
      execution_target: "kernel",
      surfaces: ["session"],
      children: [
        {
          id: "agent-focus",
          label: "focus",
          description: "Change the human focused agent",
          value: "/agent focus ",
          kind: "command",
          execution_target: "kernel",
          surfaces: ["session"],
        },
      ],
    },
  ],
}

function fakeClient() {
  const requests: Record<string, unknown>[] = []
  return {
    requests,
    send: async (request: Record<string, unknown>): Promise<Record<string, unknown>> => {
      requests.push(request)
      if ("GetTerminalCommandCatalog" in request) return { TerminalCommandCatalog: { catalog } }
      return {}
    },
  }
}

test("agent terminal searches a bounded catalog and describes stable ids", async () => {
  const client = fakeClient()
  const terminal = new AgentTerminal(client)
  const search = await terminal.search({ query: "active sessions", limit: 1 })
  assert.equal(search.revision, "catalog-test")
  assert.equal(search.results.length, 1)
  assert.equal(search.results[0]?.id, "session-list")
  assert.equal((await terminal.describe("session-list")).command.id, "session-list")
  await assert.rejects(
    () => terminal.describe("agent-focus"),
    /not supported by agent terminals/,
  )
  assert.equal(client.requests.length, 2, "registry fallback should be cached for search and describe")
})

test("agent terminal search tolerates bounded token typos without dumping the catalog", async () => {
  const terminal = new AgentTerminal(fakeClient())
  const search = await terminal.search({ query: "sessons", limit: 1 })
  assert.equal(search.results.length, 1)
  assert.equal(search.results[0]?.id, "session-list")
  assert.equal(search.results[0]?.score, 0.25)
})

test("agent terminal requires explicit context and returns updated context without server state", async () => {
  const terminal = new AgentTerminal(fakeClient())
  const execution = await terminal.execute("pwd", {
    workspace: "/repo",
    worktree: "/repo/worktree",
    workspace_id: "workspace-kernel-id",
    worktree_id: "worktree-kernel-id",
    session_id: "session-1",
    attachment_id: "attachment-agent",
    agent_id: "agent-1",
  })
  assert.equal(execution.ok, true)
  assert.match(execution.output, /repo\/worktree/)
  assert.equal(execution.context.workspace_id, "workspace-kernel-id")
  assert.equal(execution.context.worktree_id, "worktree-kernel-id")
  assert.equal(execution.context.session_id, "session-1")
  await assert.rejects(
    () => terminal.execute("pwd", { workspace: "", worktree: "/repo" }),
    /requires workspace and worktree/,
  )
})

test("agent terminal executes parity operations through their kernel request variant", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return {
          TerminalOperationRegistry: {
            registry: {
              revision: "registry-test",
              operations: [{
                id: "terminal.get_terminal_operation_registry",
                description: "Get operation registry",
                required_context: ["workspace", "worktree"],
                required_targets: [],
                input_schema: { type: "object" },
                result_kind: "registry",
                mutation: false,
                supported_surfaces: ["agent_terminal"],
                parity_variants: ["GetTerminalOperationRegistry"],
                presentation_only: false,
              }, {
                id: "terminal.submit_prompt",
                description: "Submit prompt",
                required_context: ["workspace", "worktree"],
                required_targets: ["session_id", "agent_id"],
                input_schema: { type: "object" },
                result_kind: "prompt",
                mutation: true,
                supported_surfaces: ["agent_terminal"],
                parity_variants: ["SubmitPrompt"],
                presentation_only: false,
              }],
            },
          },
        }
      }
      return { TerminalOperationRegistry: { registry: { revision: "registry-test", operations: [] } } }
    },
  }
  const result = await new AgentTerminal(client).executeOperation(
    "terminal.get_terminal_operation_registry",
    undefined,
    { workspace: "/repo", worktree: "/repo" },
  )
  assert.equal(result.ok, true)
  assert.deepEqual(requests.at(-1), { GetTerminalOperationRegistry: {} })
  await new AgentTerminal(client).executeOperation(
    "terminal.submit_prompt",
    { prompt: "hello", attachments: [] },
    { workspace: "/repo", worktree: "/repo", session_id: "s", attachment_id: "a", agent_id: "agent-1" },
  )
  assert.deepEqual(requests.at(-1), { SubmitPrompt: { prompt: "hello", attachments: [], session_id: "s", target_agent_id: "agent-1", prompt_source: "agent_terminal" } })
})

test("agent terminal keeps filesystem paths separate from kernel workspace targets", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.list_workspace_worktrees",
            description: "List worktrees",
            required_context: ["workspace", "worktree"],
            required_targets: ["workspace_id"],
            input_schema: { type: "object", additionalProperties: false, properties: { workspace_id: { type: "string" } } },
            result_kind: "kernel_response",
            mutation: false,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["ListWorkspaceWorktrees"],
            presentation_only: false,
          }],
        } } }
      }
      return { Worktrees: [] }
    },
  }
  const terminal = new AgentTerminal(client)
  await terminal.executeOperation("terminal.list_workspace_worktrees", {}, {
    workspace: "/filesystem/workspace",
    worktree: "/filesystem/worktree",
    workspace_id: "workspace-kernel-id",
    worktree_id: "worktree-kernel-id",
  })
  assert.deepEqual(requests.at(-1), { ListWorkspaceWorktrees: { workspace_id: "workspace-kernel-id" } })
  await assert.rejects(
    () => terminal.executeOperation("terminal.list_workspace_worktrees", {}, {
      workspace: "/filesystem/workspace",
      worktree: "/filesystem/worktree",
    }),
    /requires explicit workspace_id/,
  )
})

test("agent terminal rejects a stale registry revision before dispatch", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-current",
          operations: [{
            id: "terminal.get_daemon_health",
            description: "Health",
            required_context: ["workspace", "worktree"],
            required_targets: [],
            input_schema: { type: "null" },
            result_kind: "health",
            mutation: false,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["GetDaemonHealth"],
            presentation_only: false,
          }],
        } } }
      }
      return {}
    },
  }
  const terminal = new AgentTerminal(client)
  await assert.rejects(
    () => terminal.executeOperation("terminal.get_daemon_health", undefined, { workspace: "/repo", worktree: "/repo" }, { registry_revision: "registry-old" }),
    /registry revision mismatch/,
  )
  assert.equal(requests.some((request) => "GetDaemonHealth" in request), false)
})

test("agent terminal refreshes the registry before revision-gated dispatch", async () => {
  const requests: Record<string, unknown>[] = []
  let registryReads = 0
  const operation = {
    id: "terminal.get_daemon_health",
    description: "Health",
    required_context: ["workspace", "worktree"],
    required_targets: [],
    input_schema: { type: "null" },
    result_kind: "health",
    mutation: false,
    supported_surfaces: ["agent_terminal"],
    parity_variants: ["GetDaemonHealth"],
    presentation_only: false,
  }
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        registryReads += 1
        return { TerminalOperationRegistry: { registry: { revision: registryReads === 1 ? "registry-a" : "registry-b", operations: [operation] } } }
      }
      return {}
    },
  }
  const terminal = new AgentTerminal(client)
  await terminal.search()
  await assert.rejects(
    () => terminal.executeOperation("terminal.get_daemon_health", undefined, { workspace: "/repo", worktree: "/repo" }, { registry_revision: "registry-a" }),
    /registry revision mismatch/,
  )
  assert.equal(requests.some((request) => "GetDaemonHealth" in request), false)
  assert.equal(registryReads, 2)
})

test("agent terminal makes explicit context authoritative and owns prompt provenance", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.submit_prompt",
            description: "Submit prompt",
            required_context: ["workspace", "worktree"],
            required_targets: ["session_id", "agent_id"],
            input_schema: { type: "object" },
            result_kind: "prompt",
            mutation: true,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["SubmitPrompt"],
            presentation_only: false,
          }],
        } } }
      }
      return {}
    },
  }
  await new AgentTerminal(client).executeOperation("terminal.submit_prompt", {
    prompt: "hello",
    session_id: "other-session",
    target_agent_id: "other-agent",
    prompt_source: "provider_external",
  }, { workspace: "/repo", worktree: "/repo", session_id: "session-a", attachment_id: "attachment-a", agent_id: "agent-a" })
  assert.deepEqual(requests.at(-1), { SubmitPrompt: {
    prompt: "hello",
    session_id: "session-a",
    target_agent_id: "agent-a",
    prompt_source: "agent_terminal",
  } })
})

test("agent terminal owns structured attachment identity and capability", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.attach_to_session",
            description: "Attach",
            required_context: ["workspace", "worktree"],
            required_targets: ["session_id"],
            input_schema: {
              type: "object",
              additionalProperties: false,
              required: ["session_id", "client_id", "capability_level"],
              properties: {
                session_id: { type: "string" },
                client_id: { type: "string" },
                capability_level: { type: "string" },
              },
            },
            result_kind: "attachment",
            mutation: true,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["AttachToSession"],
            presentation_only: false,
          }],
        } } }
      }
      return { SessionAttached: { attachment: { id: "attachment-agent" } } }
    },
  }
  const terminal = new AgentTerminal(client, "stable-agent-client")
  await terminal.executeOperation("terminal.attach_to_session", {
    session_id: "spoof-session",
    client_id: "human-client",
    capability_level: "ReadOnly",
  }, { workspace: "/repo", worktree: "/repo", session_id: "session-a" })
  assert.deepEqual(requests.at(-1), { AttachToSession: {
    session_id: "session-a",
    client_id: "stable-agent-client:session-a",
    capability_level: "FullTerminal",
  } })
})

test("agent terminal keeps session attachments independent and closes the replacement set", async () => {
  const requests: Record<string, unknown>[] = []
  let attachmentNumber = 0
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.get_session_state",
            description: "Read session state",
            required_context: ["workspace", "worktree"],
            required_targets: ["session_id"],
            input_schema: { type: "object", additionalProperties: false, properties: { session_id: { type: "string" } }, required: ["session_id"] },
            result_kind: "session",
            mutation: false,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["GetSessionState"],
            presentation_only: false,
          }],
        } } }
      }
      if ("AttachToSession" in request) {
        attachmentNumber += 1
        return { SessionAttached: { attachment: { id: `attachment-${attachmentNumber}` } } }
      }
      if ("GetSessionState" in request) return { SessionState: { session: {} } }
      return {}
    },
  }
  const terminal = new AgentTerminal(client, "multi-session-client")
  const context = (session_id: string) => ({ workspace: "/repo", worktree: "/repo", session_id })
  await terminal.executeOperation("terminal.get_session_state", {}, context("session-a"))
  await terminal.executeOperation("terminal.get_session_state", {}, context("session-b"))
  await terminal.executeOperation("terminal.get_session_state", {}, context("session-a"))
  const attachments = requests.filter((request) => "AttachToSession" in request).map((request) => request.AttachToSession)
  assert.deepEqual(attachments, [
    { session_id: "session-a", client_id: "multi-session-client:session-a", capability_level: "FullTerminal" },
    { session_id: "session-b", client_id: "multi-session-client:session-b", capability_level: "FullTerminal" },
  ])
  await terminal.close()
  assert.deepEqual(requests.filter((request) => "DetachFromSession" in request).map((request) => request.DetachFromSession), [
    { attachment_id: "attachment-1" },
    { attachment_id: "attachment-2" },
  ])
})

test("structured attach records the returned attachment for close without an implicit prior attach", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.attach_to_session",
            description: "Attach",
            required_context: ["workspace", "worktree"],
            required_targets: ["session_id"],
            input_schema: { type: "object", additionalProperties: false, properties: { session_id: { type: "string" }, client_id: { type: "string" }, capability_level: { type: "string" } }, required: ["session_id", "client_id", "capability_level"] },
            result_kind: "attachment",
            mutation: true,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["AttachToSession"],
            presentation_only: false,
          }],
        } } }
      }
      if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "structured-attachment" } } }
      return {}
    },
  }
  const terminal = new AgentTerminal(client, "structured-client")
  const result = await terminal.executeOperation("terminal.attach_to_session", {}, { workspace: "/repo", worktree: "/repo", session_id: "session-a" })
  assert.equal(result.context.attachment_id, "structured-attachment")
  assert.equal(requests.filter((request) => "AttachToSession" in request).length, 1)
  await terminal.close()
  assert.deepEqual(requests.at(-1), { DetachFromSession: { attachment_id: "structured-attachment" } })
})

test("agent terminal reattaches once when the kernel rejects a stale attachment", async () => {
  const requests: Record<string, unknown>[] = []
  let attachmentNumber = 0
  let staleRejected = false
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.get_session_state",
            description: "Read session state",
            required_context: ["workspace", "worktree"],
            required_targets: ["session_id", "attachment_id"],
            input_schema: { type: "object", additionalProperties: false, properties: { session_id: { type: "string" }, attachment_id: { type: "string" } }, required: ["session_id", "attachment_id"] },
            result_kind: "session",
            mutation: false,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["GetSessionState"],
            presentation_only: false,
          }],
        } } }
      }
      if ("AttachToSession" in request) {
        attachmentNumber += 1
        return { SessionAttached: { attachment: { id: `stale-recovery-${attachmentNumber}` } } }
      }
      if ("GetSessionState" in request && !staleRejected) {
        staleRejected = true
        throw Object.assign(new Error("attachment no longer exists"), { code: "attachment_not_found" })
      }
      return { SessionState: { session: { id: "session-a" } } }
    },
  }
  const terminal = new AgentTerminal(client, "stale-recovery-client")
  const result = await terminal.executeOperation("terminal.get_session_state", {}, {
    workspace: "/repo",
    worktree: "/repo",
    session_id: "session-a",
    attachment_id: "stale-attachment",
  })
  assert.equal(result.context.attachment_id, "stale-recovery-1")
  assert.deepEqual(requests.filter((request) => "AttachToSession" in request).map((request) => request.AttachToSession), [
    { session_id: "session-a", client_id: "stale-recovery-client:session-a", capability_level: "FullTerminal" },
  ])
  assert.deepEqual(requests.filter((request) => "GetSessionState" in request).map((request) => request.GetSessionState), [
    { session_id: "session-a", attachment_id: "stale-attachment" },
    { session_id: "session-a", attachment_id: "stale-recovery-1" },
  ])
  await terminal.close()
})

test("agent terminal makes batch prompt context authoritative and validates its item contract", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.submit_prompts",
            description: "Submit prompts",
            required_context: ["workspace", "worktree"],
            required_targets: ["session_id", "attachment_id"],
            input_schema: {
              type: "object",
              additionalProperties: false,
              required: ["session_id", "attachment_id", "prompts"],
              properties: {
                session_id: { type: "string" },
                attachment_id: { type: "string" },
                prompts: {
                  type: "array",
                  items: {
                    type: "object",
                    additionalProperties: false,
                    required: ["target_agent_id", "prompt"],
                    properties: {
                      session_id: { type: "string" },
                      attachment_id: { type: "string" },
                      target_agent_id: { type: "string" },
                      prompt: { type: "string" },
                      prompt_source: { type: "string", const: "agent_terminal" },
                    },
                  },
                },
              },
            },
            result_kind: "prompt_batch",
            mutation: true,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["SubmitPrompts"],
            presentation_only: false,
          }],
        } } }
      }
      return {}
    },
  }
  await new AgentTerminal(client).executeOperation("terminal.submit_prompts", {
    session_id: "spoof-session",
    attachment_id: "spoof-attachment",
    prompts: [{ session_id: "spoof-session", attachment_id: "spoof-attachment", target_agent_id: "spoof-agent", prompt: "hello", prompt_source: "provider_external" }],
  }, { workspace: "/repo", worktree: "/repo", session_id: "session-a", attachment_id: "attachment-a", agent_id: "agent-a" })
  assert.deepEqual(requests.at(-1), { SubmitPrompts: {
    session_id: "session-a",
    attachment_id: "attachment-a",
    prompts: [{ session_id: "session-a", attachment_id: "attachment-a", target_agent_id: "agent-a", prompt: "hello", prompt_source: "agent_terminal" }],
  } })
  await assert.rejects(
    () => new AgentTerminal(client).executeOperation("terminal.submit_prompts", { prompts: [{ target_agent_id: "agent-a", prompt: "hello" }] }, { workspace: "/repo", worktree: "/repo", session_id: "session-a", attachment_id: "attachment-a" }),
    /requires explicit agent_id context/,
  )
})

test("agent terminal wait honors batch prompt acceptance freshness", async () => {
  const requests: Record<string, unknown>[] = []
  let stateReads = 0
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.submit_prompts",
            description: "Submit prompts",
            required_context: ["workspace", "worktree"],
            required_targets: ["session_id", "attachment_id"],
            input_schema: { type: "object", additionalProperties: false, required: ["session_id", "attachment_id", "prompts"], properties: {
              session_id: { type: "string" }, attachment_id: { type: "string" }, prompts: { type: "array", items: { type: "object" } },
            } },
            result_kind: "prompt_batch",
            mutation: true,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["SubmitPrompts"],
            presentation_only: false,
          }],
        } } }
      }
      if ("SubmitPrompts" in request) {
        return { PromptsSubmitted: { results: [{ agent_id: "agent-a", outcome: { Queued: { prompt: { id: "batch-prompt-1" } } } }], agent_activity_revision: 5 } }
      }
      if ("GetSessionState" in request) {
        stateReads += 1
        return { SessionState: { agent_activity_revision: stateReads === 1 ? 5 : 6, session: { agent_activity: { "agent-a": { busy: false, status: "idle" } } } } }
      }
      return {}
    },
  }
  const terminal = new AgentTerminal(client)
  const context = { workspace: "/repo", worktree: "/repo", session_id: "session-a", attachment_id: "attachment-a", agent_id: "agent-a" }
  await terminal.executeOperation("terminal.submit_prompts", { prompts: [{ target_agent_id: "agent-a", prompt: "hello" }] }, context)
  assert.equal((await terminal.wait(context, 0)).completed, false)
  assert.equal((await terminal.wait(context, 0)).completed, true)
})

test("agent terminal redacts credential material from structured results", async () => {
  const client = {
    send: async (request: Record<string, unknown>) => {
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.get_credential",
            description: "Get credential metadata",
            required_context: ["workspace", "worktree"],
            required_targets: [],
            input_schema: { type: "object", additionalProperties: false, required: ["id"], properties: { id: { type: "string" } } },
            result_kind: "credential",
            mutation: false,
            supported_surfaces: ["agent_terminal"],
            parity_variants: ["GetCredential"],
            presentation_only: false,
          }],
        } } }
      }
      return { Credential: { credential: { id: "github", injection: { type: "header", name: "Authorization", value: "super-secret" }, token: "another-secret" } } }
    },
  }
  const result = await new AgentTerminal(client).executeOperation("terminal.get_credential", { id: "github" }, { workspace: "/repo", worktree: "/repo" })
  assert.doesNotMatch(result.output, /super-secret|another-secret/)
  assert.match(result.output, /REDACTED/)
})

test("agent terminal redaction also covers sensitive error text", async () => {
  const { redactSensitiveText } = await import("./agent-terminal.js")
  assert.equal(redactSensitiveText("provider failed token=secret-value"), "provider failed token=[REDACTED]")
  assert.equal(
    redactSensitiveText('{"token":"secret-value","cloud_invite":"cloud-secret","local_invite":local-secret}'),
    '{"token":"[REDACTED]","cloud_invite":"[REDACTED]","local_invite":[REDACTED]}',
  )
})

test("agent terminal does not execute kernel-internal registry entries", async () => {
  const client = {
    send: async (request: Record<string, unknown>) => {
      if ("GetTerminalOperationRegistry" in request) {
        return { TerminalOperationRegistry: { registry: {
          revision: "registry-test",
          operations: [{
            id: "terminal.complete_prompt",
            description: "Kernel callback",
            required_context: [],
            required_targets: [],
            input_schema: { type: "object" },
            result_kind: "kernel_response",
            mutation: true,
            supported_surfaces: ["tui"],
            parity_variants: ["CompletePrompt"],
            presentation_only: false,
          }],
        } } }
      }
      return {}
    },
  }
  const terminal = new AgentTerminal(client)
  assert.deepEqual((await terminal.search({ query: "complete" })).results, [])
  await assert.rejects(
    () => terminal.executeOperation("terminal.complete_prompt", {}, { workspace: "/repo", worktree: "/repo" }),
    /not supported by agent terminals/,
  )
})

test("agent terminal never mutates human focus", async () => {
  const terminal = new AgentTerminal(fakeClient())
  await assert.rejects(
    () => terminal.execute("agent focus agent-2", { workspace: "/repo", worktree: "/repo", session_id: "session-1" }),
    /cannot change human focus/,
  )
})

test("agent terminal applies focus and target validation to sourced commands", async () => {
  const terminal = new AgentTerminal(fakeClient())
  const focus = await terminal.execute("source nested.chariox", { workspace: "/repo", worktree: "/repo" }, {
    shell: { loadScript: async () => "agent focus agent-2\n" },
  })
  assert.equal(focus.ok, false)
  assert.match(focus.output, /cannot change human focus/)
  assert.doesNotMatch(focus.output, /@ agent focus/)
  const prompt = await terminal.execute("run nested.chariox", { workspace: "/repo", worktree: "/repo", session_id: "s", attachment_id: "a" }, {
    shell: { loadScript: async () => "prompt summarize this\n" },
  })
  assert.equal(prompt.ok, false)
  assert.match(prompt.output, /explicit agent_id/)
})

test("agent terminal keeps sourced scripts inside the selected worktree", async () => {
  const terminal = new AgentTerminal(fakeClient())
  await assert.rejects(
    () => terminal.execute("source /tmp/outside.chariox", { workspace: "/repo", worktree: "/repo/worktree" }, {
      shell: { loadScript: async () => "pwd\n" },
    }),
    /must stay inside the selected worktree/,
  )
})

test("agent terminal rejects symlinked scripts that escape the selected worktree", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-agent-terminal-"))
  const worktree = join(root, "worktree")
  const outside = join(root, "outside.chariox")
  await mkdir(worktree)
  await writeFile(outside, "pwd\n", "utf8")
  await symlink(outside, join(worktree, "link.chariox"))
  try {
    await assert.rejects(
      () => new AgentTerminal(fakeClient()).execute("source link.chariox", { workspace: root, worktree }),
      /must stay inside the selected worktree/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("agent terminal obtains its own ordinary attachment without inheriting human focus", async () => {
  const client = fakeClient()
  client.send = async (request) => {
    client.requests.push(request)
    if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "agent-attachment" } } }
    if ("ListSessions" in request) return { SessionsListed: { sessions: [] } }
    return {}
  }
  const terminal = new AgentTerminal(client)
  await terminal.execute("session list", { workspace: "/repo", worktree: "/repo", session_id: "session-1", agent_id: "agent-1" })
  const attachmentRequest = client.requests[0]?.AttachToSession as { session_id?: string; client_id?: string; capability_level?: string }
  assert.equal(attachmentRequest.session_id, "session-1")
  assert.equal(attachmentRequest.capability_level, "FullTerminal")
  assert.equal(attachmentRequest.client_id?.startsWith("chariox-agent-terminal-"), true)
})

test("agent terminal subscribes its ordinary attachment to shared kernel events", async () => {
  const subscriptions: string[] = []
  const unsubscriptions: string[] = []
  const client = {
    requests: [] as Record<string, unknown>[],
    send: async (request: Record<string, unknown>) => {
      client.requests.push(request)
      if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "agent-attachment" } } }
      return {}
    },
    subscribeToKernelEvents: async (sessionId: string, attachmentId: string) => { subscriptions.push(`${sessionId}:${attachmentId}`) },
    unsubscribeFromKernelEvents: async () => { unsubscriptions.push("closed") },
  }
  const terminal = new AgentTerminal(client, "event-client")
  await terminal.execute("pwd", { workspace: "/repo", worktree: "/repo", session_id: "session-1" })
  await terminal.execute("pwd", { workspace: "/repo", worktree: "/repo", session_id: "session-1" })
  assert.deepEqual(subscriptions, ["session-1:agent-attachment"], "attachment event subscription should be reused")
  await terminal.close()
  assert.deepEqual(unsubscriptions, ["closed"])
})

test("agent terminal keeps independent event subscriptions for parallel sessions", async () => {
  const subscriptions: string[] = []
  const unsubscriptions: string[] = []
  const forwarded: string[] = []
  const eventClients: { emit: (event: KernelEvent) => void }[] = []
  const client = {
    requests: [] as Record<string, unknown>[],
    send: async (request: Record<string, unknown>) => {
      client.requests.push(request)
      const attach = request.AttachToSession as { session_id?: string } | undefined
      if (attach?.session_id) return { SessionAttached: { attachment: { id: `${attach.session_id}-attachment` } } }
      return {}
    },
  }
  const terminal = new AgentTerminal(
    client,
    "parallel-event-client",
    () => {
      let handler: ((event: KernelEvent) => void) | undefined
      const eventClient = {
        send: async (_request: Record<string, unknown>) => ({}),
        subscribeToKernelEvents: async (sessionId: string, attachmentId: string) => {
          subscriptions.push(`${sessionId}:${attachmentId}`)
        },
        unsubscribeFromKernelEvents: async () => {
          unsubscriptions.push("closed")
        },
        onKernelEvent: (next: (event: KernelEvent) => void) => {
          handler = next
          return () => { handler = undefined }
        },
        close: async () => {},
        emit: (event: KernelEvent) => handler?.(event),
      }
      eventClients.push(eventClient)
      return eventClient
    },
  )
  terminal.onKernelEvent((event) => forwarded.push(event.event))
  await terminal.execute("pwd", { workspace: "/repo", worktree: "/repo", session_id: "session-a" })
  await terminal.execute("pwd", { workspace: "/repo", worktree: "/repo", session_id: "session-b" })
  assert.deepEqual(subscriptions, ["session-a:session-a-attachment", "session-b:session-b-attachment"])
  eventClients[0]?.emit({ event: "session_unavailable", session_id: "session-a", message: "updated" })
  eventClients[1]?.emit({ event: "session_unavailable", session_id: "session-b", message: "updated" })
  assert.deepEqual(forwarded, ["session_unavailable", "session_unavailable"])
  await terminal.close()
  assert.deepEqual(unsubscriptions, ["closed", "closed"])
})

test("agent terminal shell session switches keep attachment identities session-scoped", async () => {
  const requests: Record<string, unknown>[] = []
  let attachmentNumber = 0
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("AttachToSession" in request) {
        attachmentNumber += 1
        return { SessionAttached: { attachment: { id: `shell-attachment-${attachmentNumber}` } } }
      }
      if ("ResolveSession" in request) {
        return { SessionResolved: { session: {
          id: "session-b",
          alias: "session-b",
          workspace_id: "/repo",
          worktree_id: "/repo",
          agents: [],
          attachment_ids: [],
        } } }
      }
      return {}
    },
  }
  const terminal = new AgentTerminal(client, "shell-switch-client")
  const result = await terminal.execute("session use session-b", {
    workspace: "/repo",
    worktree: "/repo",
    session_id: "session-a",
  })
  const attachments = requests.filter((request) => "AttachToSession" in request).map((request) => request.AttachToSession)
  assert.deepEqual(attachments, [
    { session_id: "session-a", client_id: "shell-switch-client:session-a", capability_level: "FullTerminal" },
    { session_id: "session-b", client_id: "shell-switch-client:session-b", capability_level: "FullTerminal" },
  ])
  assert.equal(result.context.session_id, "session-b")
  assert.equal(result.context.attachment_id, "shell-attachment-2")
  assert.equal(result.context.workspace_id, "/repo")
  assert.equal(result.context.worktree_id, "/repo")
  await terminal.close()
  assert.deepEqual(requests.filter((request) => "DetachFromSession" in request).map((request) => request.DetachFromSession), [
    { attachment_id: "shell-attachment-1" },
    { attachment_id: "shell-attachment-2" },
  ])
})

test("agent terminal wait requires an explicit target agent", async () => {
  const terminal = new AgentTerminal(fakeClient())
  await assert.rejects(
    () => terminal.wait({ workspace: "/repo", worktree: "/repo", session_id: "s", attachment_id: "a" }),
    /explicit session_id and agent_id/,
  )
})

test("agent terminal wait pumps the shared session state without changing focus", async () => {
  const client = fakeClient()
  client.send = async (request) => {
    client.requests.push(request)
    if ("GetSessionState" in request) {
      return { SessionState: { session: { agent_activity: { "agent-1": { busy: false, status: "idle" } } } } }
    }
    return {}
  }
  const terminal = new AgentTerminal(client)
  const result = await terminal.wait({
    workspace: "/repo",
    worktree: "/repo",
    session_id: "session-1",
    attachment_id: "attachment-agent",
    agent_id: "agent-1",
  })
  assert.equal(result.completed, true)
  assert.equal(client.requests.length, 2)
})

test("agent terminal wait reads the wire-level sibling agent_activity projection", async () => {
  const client = fakeClient()
  client.send = async (request) => {
    client.requests.push(request)
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: { id: "session-1", agents: [{ id: "agent-1" }] },
          agent_activity: { "agent-1": { busy: false, status: "idle", prompt_status: "none" } },
          agent_activity_revision: 7,
        },
      }
    }
    return {}
  }
  const terminal = new AgentTerminal(client)
  const result = await terminal.wait({
    workspace: "/repo",
    worktree: "/repo",
    session_id: "session-1",
    attachment_id: "attachment-1",
    agent_id: "agent-1",
  })
  assert.equal(result.completed, true)
  assert.equal(result.agent_activity_revision, 7)
  assert.deepEqual(result.agent_activity, { "agent-1": { busy: false, status: "idle", prompt_status: "none" } })
})

test("agent terminal wait targets the named agent while another agent remains busy", async () => {
  const client = fakeClient()
  client.send = async (request) => {
    client.requests.push(request)
    if ("GetSessionState" in request) {
      return {
        SessionState: {
          session: {
            agent_activity: {
              "agent-1": { busy: false, status: "idle" },
              "agent-2": { busy: true, status: "working" },
            },
          },
        },
      }
    }
    return {}
  }
  const result = await new AgentTerminal(client).wait({
    workspace: "/repo",
    worktree: "/repo",
    session_id: "session-1",
    attachment_id: "attachment-agent",
    agent_id: "agent-1",
  })
  assert.equal(result.completed, true)
  assert.equal(result.timed_out, false)
})

test("agent terminal wait does not complete for queued work represented as idle", async () => {
  const client = fakeClient()
  client.send = async (request) => {
    client.requests.push(request)
    if ("GetSessionState" in request) {
      return { SessionState: { session: { agent_activity: { "agent-1": { busy: false, status: "idle", prompt_status: "queued", queued_prompt_count: 1 } } } } }
    }
    return {}
  }
  const result = await new AgentTerminal(client).wait({
    workspace: "/repo",
    worktree: "/repo",
    session_id: "session-1",
    attachment_id: "attachment-agent",
    agent_id: "agent-1",
  }, 0)
  assert.equal(result.completed, false)
  assert.equal(result.timed_out, true)
})

test("agent terminal wait does not trust a stale projection after submitting a prompt", async () => {
  const client = fakeClient()
  let stateReads = 0
  client.send = async (request) => {
    client.requests.push(request)
    if ("GetTerminalOperationRegistry" in request) {
      return { TerminalOperationRegistry: { registry: {
        revision: "r1",
        operations: [{
          id: "terminal.submit_prompt",
          description: "Submit prompt",
          required_context: ["workspace", "worktree"],
          required_targets: ["session_id", "agent_id"],
          input_schema: { type: "object" },
          result_kind: "prompt",
          mutation: true,
          supported_surfaces: ["agent_terminal"],
          parity_variants: ["SubmitPrompt"],
          presentation_only: false,
        }],
      } } }
    }
    if ("SubmitPrompt" in request) {
      return { PromptSubmitted: { outcome: { Queued: { prompt: { id: "prompt-1" } } }, agent_activity_revision: 5 } }
    }
    if ("GetSessionState" in request) {
      stateReads += 1
      return { SessionState: {
        agent_activity_revision: stateReads === 1 ? 5 : 6,
        session: { agent_activity: { "agent-1": { busy: false, status: "idle" } } },
      } }
    }
    return {}
  }
  const terminal = new AgentTerminal(client)
  const context = { workspace: "/repo", worktree: "/repo", session_id: "s", attachment_id: "a", agent_id: "agent-1" }
  await terminal.executeOperation("terminal.submit_prompt", { prompt: "queued" }, context)
  const stale = await terminal.wait(context, 0)
  assert.equal(stale.completed, false)
  const fresh = await terminal.wait(context, 0)
  assert.equal(fresh.completed, true)
})

test("agent terminal wait can be cancelled", async () => {
  const client = fakeClient()
  client.send = async (request) => {
    client.requests.push(request)
    if ("GetSessionState" in request) {
      return { SessionState: { session: { agent_activity: { "agent-1": { busy: true, status: "working" } } } } }
    }
    return {}
  }
  const controller = new AbortController()
  const pending = new AgentTerminal(client).wait({
    workspace: "/repo",
    worktree: "/repo",
    session_id: "session-1",
    attachment_id: "attachment-agent",
    agent_id: "agent-1",
  }, 10_000, controller.signal)
  await new Promise((resolve) => setImmediate(resolve))
  controller.abort()
  await assert.rejects(pending, /aborted/i)
})

test("agent terminal execute can be cancelled while a shell command is pending", async () => {
  const controller = new AbortController()
  const pending = new AgentTerminal(fakeClient()).execute(
    "source nested.chariox",
    { workspace: "/repo", worktree: "/repo" },
    { signal: controller.signal, shell: { loadScript: async () => await new Promise<string>(() => {}) } },
  )
  await new Promise((resolve) => setImmediate(resolve))
  controller.abort()
  await assert.rejects(pending, /aborted/i)
})

test("agent terminal wait treats an omitted timeout as bounded and zero as an immediate timeout", async () => {
  const client = fakeClient()
  client.send = async (request): Promise<Record<string, unknown>> => {
    client.requests.push(request)
    if ("GetSessionState" in request) {
      return { SessionState: { session: { agent_activity: { "agent-1": { busy: true, status: "working" } } } } }
    }
    return {}
  }
  const terminal = new AgentTerminal(client)
  const result = await terminal.wait({ workspace: "/repo", worktree: "/repo", session_id: "s", attachment_id: "a", agent_id: "agent-1" }, 0)
  assert.equal(result.timed_out, true)
})
