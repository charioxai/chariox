import assert from "node:assert/strict"
import test from "node:test"

import {
  HOSTED_HOME_PROXY_MODEL,
  startHostedRemoteProviderRun,
  withHostedKernelIsolation,
} from "./hosted-cloud-kernel-scenarios.mjs"

test("hosted kernels keep state isolated while preserving explicit provider homes", () => {
  const source = {
    HOME: "/real-home",
    CODEX_HOME: "/real-codex",
    CHARIOX_CLOUD_DEV_AUTH_SECRET: "test-secret",
  }
  const isolated = withHostedKernelIsolation(source, {
    homeDir: "/tmp/drill/home",
    charioxHome: "/tmp/drill/home/.chariox",
    xdgConfigHome: "/tmp/drill/home/.config",
    xdgStateHome: "/tmp/drill/home/.local/state",
    xdgRuntimeDir: "/tmp/drill/home/run",
  })

  assert.equal(isolated.HOME, "/tmp/drill/home")
  assert.equal(isolated.CHARIOX_HOME, "/tmp/drill/home/.chariox")
  assert.equal(isolated.XDG_CONFIG_HOME, "/tmp/drill/home/.config")
  assert.equal(isolated.XDG_STATE_HOME, "/tmp/drill/home/.local/state")
  assert.equal(isolated.XDG_RUNTIME_DIR, "/tmp/drill/home/run")
  assert.equal(isolated.CODEX_HOME, "/real-codex")
  assert.equal(isolated.CHARIOX_CLOUD_DEV_AUTH_SECRET, "test-secret")
  assert.equal(source.HOME, "/real-home")
})

test("hosted home-proxy runs use an idle managed provider model", () => {
  assert.equal(HOSTED_HOME_PROXY_MODEL, "native-tui-idle")
})

test("hosted home-proxy runtime starts through the Chariox prompt path", async () => {
  const sent = []
  const requests = {
    submitPromptRequest(sessionId, attachmentId, agentId, prompt, attachments) {
      return { SubmitPrompt: { sessionId, attachmentId, agentId, prompt, attachments } }
    },
    listAgentsRequest(sessionId) {
      return { ListAgents: { sessionId } }
    },
    getProviderRunRequest(providerRunId) {
      return { GetProviderRun: { providerRunId } }
    },
  }
  const client = {
    async send(request) {
      sent.push(request)
      if (request.ListAgents) {
        return {
          AgentsListed: {
            agents: [{
              id: "agent-1",
              remote_execution: {
                leased_agent_id: "lease-agent-1",
                active_worker_provider_run_id: "worker-run-1",
              },
            }],
          },
        }
      }
      if (request.GetProviderRun) {
        return {
          ProviderRun: {
            provider_run: {
              id: "leased:lease-agent-1:worker-run-1",
              runtime_mcp_server_url: "http://127.0.0.1:9000/mcp",
              runtime_mcp_auth_token: "test-token",
            },
          },
        }
      }
      return { PromptSubmitted: {} }
    },
  }

  const run = await startHostedRemoteProviderRun({
    client,
    requests,
    sessionId: "session-1",
    attachmentId: "attachment-1",
    agentId: "agent-1",
    prompt: "initialize",
  })

  assert.equal(run.id, "leased:lease-agent-1:worker-run-1")
  assert.deepEqual(sent.map((request) => Object.keys(request)[0]), [
    "SubmitPrompt",
    "ListAgents",
    "GetProviderRun",
  ])
  assert.equal(sent.some((request) => "LaunchProviderRun" in request), false)
})
