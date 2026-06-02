import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  ProviderProcessInfo,
  WorkflowPublicationTrustedSender,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand reports relay status", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { RelayStatus: null })
    return {
      RelayStatus: {
        status: {
          configured: true,
          connected: false,
          relay_url: "wss://relay.example",
          relay_token_configured: true,
          daemon_id: "daemon-1",
          machine_id: "machine-1",
          machine_alias: "mini",
        },
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("relay status"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /configured, disconnected/)
  assert.match(result.message ?? "", /machine=mini/)
})

test("executeShellCommand lists MCP servers and skills in the workspace", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListMcpServers" in request) {
          return { McpServersListed: { mcps: [{ name: "playwright", transport: { stdio: { command: "npx" } }, enabled: true }] } }
        }
        return { SkillsListed: { skills: [{ name: "qa", description: "QA checks", path: "/skills/qa" }] } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const mcpResult = await executeShellCommand(parseShellCommand("mcp list"), context, { client: fake.client })
  const skillResult = await executeShellCommand(parseShellCommand("skill list"), context, { client: fake.client })
  assert.equal(mcpResult.ok, true)
  assert.match(mcpResult.message ?? "", /playwright \[enabled\]/)
  assert.equal(skillResult.ok, true)
  assert.match(skillResult.message ?? "", /qa - QA checks/)
  assert.deepEqual(requests, [
    { ListMcpServers: { workspace_id: "/repo" } },
    { ListSkills: { workspace_id: "/repo" } },
  ])
})

test("executeShellCommand shows config and provider auth status", async () => {
  const fake = fakeClient((request) => {
    if ("GetUserConfig" in request) {
      return { UserConfig: { path: "/home/.arroba/config.json", config: { version: 1, providers: { default: "codex" } } } }
    }
    assert.deepEqual(request, { GetProviderAuthStatus: { provider: "codex" } })
    return {
      ProviderAuthStatus: {
        status: {
          provider: "codex",
          auth_state: "authenticated",
          account_profile: "default",
          login_hint: null,
          detected_version: "1.2.3",
        },
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", provider: "codex" })
  const configResult = await executeShellCommand(parseShellCommand("config show"), context, { client: fake.client })
  const providerResult = await executeShellCommand(parseShellCommand("provider status"), context, { client: fake.client })
  assert.equal(configResult.ok, true)
  assert.match(configResult.message ?? "", /"default": "codex"/)
  assert.equal(providerResult.ok, true)
  assert.match(providerResult.message ?? "", /codex: authenticated as default/)
  assert.match(providerResult.message ?? "", /version 1.2.3/)
})

test("executeShellCommand mutates user config", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GetUserConfig" in request) {
          return { UserConfig: { path: "/home/.arroba/config.json", config: { version: 1, providers: { default: "codex" } } } }
        }
        if ("GetUserConfigSchema" in request) {
          return {
            UserConfigSchema: {
              entries: [
                {
                  path: "providers.workspace_live_sync",
                  value_type: "enum",
                  allowed_values: ["off", "managed", "tracked"],
                  settable: true,
                  unsettable: true,
                  effect: "provider_reload",
                  status: "live",
                  description: "Global workspace live sync policy.",
                },
              ],
            },
          }
        }
        const setConfig = "SetUserConfigValue" in request
          ? request.SetUserConfigValue as { path?: string }
          : null
        return {
          UserConfigUpdated: {
            path: "/home/.arroba/config.json",
            config: { version: 1, providers: { workspace_live_sync: "off" } },
            effects: setConfig?.path === "providers.workspace_live_sync"
              ? [
                  {
                    kind: "provider_reload",
                    path: "providers.workspace_live_sync",
                    message: "workspace live sync policy updated; provider reloads: 1 reloaded, 0 deferred, 0 unaffected",
                    provider_reload: { reloaded: 1, deferred: 0, unaffected: 0 },
                  },
                ]
              : [],
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const pathResult = await executeShellCommand(parseShellCommand("config path"), context, { client: fake.client })
  const keysResult = await executeShellCommand(parseShellCommand("config keys"), context, { client: fake.client })
  const schemaResult = await executeShellCommand(parseShellCommand("config schema"), context, { client: fake.client })
  const setResult = await executeShellCommand(parseShellCommand("config set providers.default opencode"), context, { client: fake.client })
  const unsetResult = await executeShellCommand(parseShellCommand("config unset providers.default"), context, { client: fake.client })
  const offWorkspaceLiveSyncResult = await executeShellCommand(parseShellCommand("config workspace-live-sync off"), context, { client: fake.client })
  const managedWorkspaceLiveSyncResult = await executeShellCommand(parseShellCommand("config workspace-live-sync managed"), context, { client: fake.client })
  const defaultWorkspaceLiveSyncResult = await executeShellCommand(parseShellCommand("config workspace-live-sync"), context, { client: fake.client })
  assert.equal(pathResult.ok, true)
  assert.equal(pathResult.message, "/home/.arroba/config.json")
  assert.equal(keysResult.ok, true)
  assert.match(keysResult.message ?? "", /providers\.workspace_live_sync/)
  assert.equal(schemaResult.ok, true)
  assert.equal(schemaResult.format, "json")
  assert.match(schemaResult.message ?? "", /provider_reload/)
  assert.equal(setResult.ok, true)
  assert.match(setResult.message ?? "", /config providers.default set to opencode/)
  assert.equal(unsetResult.ok, true)
  assert.match(unsetResult.message ?? "", /config providers.default unset/)
  assert.equal(offWorkspaceLiveSyncResult.ok, true)
  assert.match(offWorkspaceLiveSyncResult.message ?? "", /default workspace live sync for new sessions disabled; other repositories remain unrestricted/)
  assert.equal(managedWorkspaceLiveSyncResult.ok, true)
  assert.match(managedWorkspaceLiveSyncResult.message ?? "", /default workspace live sync for new sessions set to managed \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.equal(defaultWorkspaceLiveSyncResult.ok, true)
  assert.match(defaultWorkspaceLiveSyncResult.message ?? "", /default workspace live sync for new sessions disabled; other repositories remain unrestricted/)
  assert.deepEqual(requests, [
    { GetUserConfig: null },
    { GetUserConfigSchema: null },
    { GetUserConfigSchema: null },
    { SetUserConfigValue: { path: "providers.default", value: "opencode" } },
    { UnsetUserConfigValue: { path: "providers.default" } },
    { SetUserConfigValue: { path: "providers.workspace_live_sync", value: "off" } },
    { SetUserConfigValue: { path: "providers.workspace_live_sync", value: "managed" } },
    { SetUserConfigValue: { path: "providers.workspace_live_sync", value: "off" } },
  ])
})

test("executeShellCommand stores credentials only through hidden reader", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListCredentials" in request) {
          return {
            CredentialsListed: {
              credentials: [
                {
                  id: "github",
                  source: { type: "vault", key: "github-token" },
                  allowed_uses: ["connector"],
                },
              ],
            }
          }
        }
        if ("SetCredentialSecret" in request) {
          return { CredentialSecretStored: { key: "github-token" } }
        }
        return { CredentialSecretDeleted: { key: "github-token" } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const listResult = await executeShellCommand(parseShellCommand("credential list"), context, { client: fake.client })
  const blockedResult = await executeShellCommand(parseShellCommand("credential set github-token"), context, { client: fake.client })
  const setResult = await executeShellCommand(parseShellCommand("credential set github-token"), context, {
    client: fake.client,
    readSecret: async () => "hidden-secret",
  })
  const deleteResult = await executeShellCommand(parseShellCommand("credential delete github-token"), context, { client: fake.client })

  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /github\tvault\tconnector/)
  assert.equal(blockedResult.ok, false)
  assert.match(blockedResult.message ?? "", /hidden input/)
  assert.equal(setResult.ok, true)
  assert.equal(deleteResult.ok, true)
  assert.deepEqual(requests, [
    { ListCredentials: null },
    { SetCredentialSecret: { key: "github-token", value: "hidden-secret" } },
    { DeleteCredentialSecret: { key: "github-token" } },
  ])
})

test("executeShellCommand installs and updates MCP servers", async () => {
  const installed: ArrobaMcpServerConfig = {
    name: "playwright",
    transport: { type: "stdio", command: "npx", args: ["@playwright/mcp"], env: {}, env_vars: ["GITHUB_TOKEN"] },
    enabled: true,
    required: false,
  }
  const updated: ArrobaMcpServerConfig = {
    name: "browser",
    transport: {
      type: "streamable_http",
      url: "https://mcp.example",
      bearer_token_env_var: "MCP_TOKEN",
      http_headers: {},
      env_http_headers: {},
    },
    enabled: true,
    required: false,
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InstallMcpServer" in request) {
          return { McpServerInstalled: { mcp: installed } }
        }
        return { McpServerUpdated: { mcp: updated } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const installResult = await executeShellCommand(
    parseShellCommand("mcp install playwright --command npx --arg @playwright/mcp --env GITHUB_TOKEN"),
    context,
    { client: fake.client },
  )
  const updateResult = await executeShellCommand(
    parseShellCommand("mcp update browser --url https://mcp.example --bearer-token-env-var MCP_TOKEN"),
    context,
    { client: fake.client },
  )
  assert.equal(installResult.ok, true)
  assert.match(installResult.message ?? "", /installed MCP playwright/)
  assert.equal(updateResult.ok, true)
  assert.match(updateResult.message ?? "", /updated MCP browser/)
  assert.deepEqual(requests, [
    {
      InstallMcpServer: {
        workspace_id: "/repo",
        config: installed,
      },
    },
    {
      UpdateMcpServer: {
        workspace_id: "/repo",
        config: updated,
      },
    },
  ])
})

test("executeShellCommand imports MCP servers and skills", async () => {
  const mcp: ArrobaMcpServerConfig = {
    name: "github",
    transport: { type: "stdio", command: "github-mcp-server", args: [], env: {}, env_vars: [] },
    enabled: true,
    required: false,
  }
  const skill: ArrobaSkillMetadata = {
    name: "qa",
    description: "QA checks",
    short_description: "QA",
    path: "/skills/qa",
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ImportMcpServers" in request) {
          return {
            McpServersImported: {
              outcome: {
                imported: [mcp],
                skipped: [{ name: "oauth", reason: "oauth transport is provider-native" }],
              },
            },
          }
        }
        return {
          SkillsImported: {
            outcome: {
              imported: [skill],
              skipped: [{ name: "old", reason: "already installed" }],
            },
          },
        }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const mcpResult = await executeShellCommand(parseShellCommand("mcp import codex github"), context, { client: fake.client })
  const skillResult = await executeShellCommand(parseShellCommand("skill import codex qa"), context, { client: fake.client })
  assert.equal(mcpResult.ok, true)
  assert.match(mcpResult.message ?? "", /Imported MCPs: github/)
  assert.match(mcpResult.message ?? "", /oauth: oauth transport is provider-native/)
  assert.equal(skillResult.ok, true)
  assert.match(skillResult.message ?? "", /Imported skills: qa/)
  assert.match(skillResult.message ?? "", /old: already installed/)
  assert.deepEqual(requests, [
    { ImportMcpServers: { workspace_id: "/repo", provider: "codex", name: "github" } },
    { ImportSkills: { workspace_id: "/repo", provider: "codex", name: "qa" } },
  ])
})

test("executeShellCommand grants, revokes, and lists agent extensions", async () => {
  const agent = makeAgent({
    extension_grants: [
      { kind: "mcp", name: "playwright" },
      { kind: "skill", name: "qa" },
    ],
  })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GrantAgentExtension" in request) {
          return { AgentExtensionGranted: { agent } }
        }
        if ("RevokeAgentExtension" in request) {
          return { AgentExtensionRevoked: { agent } }
        }
        return { AgentsListed: { agents: [agent] } }
      },
    },
  }
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const grantResult = await executeShellCommand(parseShellCommand("mcp grant agent-1 playwright"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("skill revoke agent-1 qa"), context, { client: fake.client })
  const grantsResult = await executeShellCommand(parseShellCommand("mcp grants"), context, { client: fake.client })
  assert.equal(grantResult.ok, true)
  assert.match(grantResult.message ?? "", /granted MCP playwright to agent-1/)
  assert.deepEqual(grantResult.contextUpdates, { agentId: "agent-1" })
  assert.equal(revokeResult.ok, true)
  assert.match(revokeResult.message ?? "", /revoked skill qa from agent-1/)
  assert.equal(grantsResult.ok, true)
  assert.match(grantsResult.message ?? "", /agent-1 MCP grants/)
  assert.match(grantsResult.message ?? "", /playwright/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    { GrantAgentExtension: { workspace_id: "/repo", agent_ref: "agent-1", kind: "mcp", name: "playwright" } },
    { RevokeAgentExtension: { agent_ref: "agent-1", kind: "skill", name: "qa" } },
    { ListAgents: { session_id: "session-1" } },
  ])
})

test("executeShellCommand confirms active home-proxy grants before exposing home execution", async () => {
  const agent = makeAgent({
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GrantAgentExtension" in request) return { AgentExtensionGranted: { agent } }
        return { AgentsListed: { agents: [agent] } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", agentId: "agent-1" })

  const blocked = await executeShellCommand(parseShellCommand("extension grant script agent-1 lookup --env py"), context, { client: fake.client })
  const confirmed = await executeShellCommand(parseShellCommand("extension grant script agent-1 lookup --env py --confirm-home-proxy"), context, { client: fake.client })

  assert.equal(blocked.ok, false)
  assert.match(blocked.message ?? "", /Confirm exposing script lookup to remote agent agent-1; home keeps credentials local and executes calls on this machine\./)
  assert.match(blocked.message ?? "", /rerun: extension grant script agent-1 lookup --env py --confirm-home-proxy/)
  assert.equal(confirmed.ok, true)
  assert.match(confirmed.message ?? "", /granted script lookup to agent-1/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    { ListAgents: { session_id: "session-1" } },
    { GrantAgentExtension: { workspace_id: "/repo", agent_ref: "agent-1", kind: "script", name: "lookup", environment: "py" } },
  ])
})

test("executeShellCommand grants passive remote skills without home-proxy confirmation", async () => {
  const agent = makeAgent({
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return { AgentExtensionGranted: { agent } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", agentId: "agent-1" })

  const result = await executeShellCommand(parseShellCommand("extension grant skill agent-1 qa"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests, [
    { GrantAgentExtension: { workspace_id: "/repo", agent_ref: "agent-1", kind: "skill", name: "qa" } },
  ])
})

test("executeShellCommand shows remote extension sync diagnostics", async () => {
  const agent = makeAgent({
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
    remote_extension_manifest_sync: {
      state: "failed",
      manifest_hash: "hash-1",
      last_error: "relay offline",
      pending_revoke: true,
    },
    extension_grants: [{ kind: "script", name: "lookup", environment: "py" }],
  })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("SyncRemoteExtensionManifest" in request) {
          return { RemoteExtensionManifestSynced: { agent } }
        }
        if ("ListHomeExtensionAudit" in request) {
          return {
            HomeExtensionAuditListed: {
              events: [{
                kind: "home_extension.invoke.denied",
                timestamp_ms: 1700000000000,
                payload: {
                  home_user_id: "alice",
                  caller_user_id: "bob",
                  agent_id: "agent-1",
                  lease_id: "lease-1",
                  worker_kernel_id: "worker-1",
                  worker_provider_run_id: "run-1",
                  status: "denied",
                  error: "worker mismatch",
                  duration_ms: 24,
                  result_bytes: 0,
                  ok: false,
                  args: { secret: "not rendered" },
                  result: { body: "not rendered" },
                  invocation: {
                    invocation_id: "invoke-1",
                    provider_tool_call_id: "call-1",
                    attempt: 2,
                    idempotency_key: "idem-1",
                  },
                  tool: {
                    kind: "script",
                    name: "lookup",
                    tool_name: "lookup",
                    safety: "read",
                    timeout_sec: 30,
                    version_hash: "hash-tool-1",
                  },
                },
              }],
            },
          }
        }
        return { AgentsListed: { agents: [agent] } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", agentId: "agent-1" })
  const grantsResult = await executeShellCommand(parseShellCommand("extension grants script"), context, { client: fake.client })
  const statusResult = await executeShellCommand(parseShellCommand("extension sync-status"), context, { client: fake.client })
  const retryResult = await executeShellCommand(parseShellCommand("extension sync-retry agent-1"), context, { client: fake.client })
  const auditResult = await executeShellCommand(parseShellCommand("extension audit agent-1 --limit 1"), context, { client: fake.client })

  assert.equal(grantsResult.ok, true)
  assert.match(grantsResult.message ?? "", /home-proxy/)
  assert.match(grantsResult.message ?? "", /remote extension sync: failed, pending revoke, relay offline/)
  assert.match(grantsResult.message ?? "", /runtime: home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools/)
  assert.match(grantsResult.message ?? "", /authority boundary: home validates every call; credentials never leave home/)
  assert.match(grantsResult.message ?? "", /placement: remote \(worker=machine-1, kernel=worker-1, lease=lease-1, leased_agent=leased-agent-1\)/)
  assert.match(grantsResult.message ?? "", /next: keep the home revoke in place; run \/extension sync-status agent-1; run \/machine kernels machine-1 if the revoke stays pending; use \/extension sync-retry agent-1 after the worker reconnects/)
  assert.equal(statusResult.ok, true)
  assert.match(statusResult.message ?? "", /runtime: home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools/)
  assert.match(statusResult.message ?? "", /authority boundary: home validates every call; credentials never leave home/)
  assert.match(statusResult.message ?? "", /worker kernel: worker-1/)
  assert.match(statusResult.message ?? "", /placement: remote \(worker=machine-1, kernel=worker-1, lease=lease-1, leased_agent=leased-agent-1\)/)
  assert.match(statusResult.message ?? "", /execution lease: lease-1/)
  assert.match(statusResult.message ?? "", /next: keep the home revoke in place; run \/extension sync-status agent-1; run \/machine kernels machine-1 if the revoke stays pending; use \/extension sync-retry agent-1 after the worker reconnects/)
  assert.equal(retryResult.ok, true)
  assert.match(retryResult.message ?? "", /runtime: home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools/)
  assert.match(retryResult.message ?? "", /authority boundary: home validates every call; credentials never leave home/)
  assert.match(retryResult.message ?? "", /manifest hash: hash-1/)
  assert.match(retryResult.message ?? "", /next: keep the home revoke in place; run \/extension sync-status agent-1; run \/machine kernels machine-1 if the revoke stays pending; use \/extension sync-retry agent-1 after the worker reconnects/)
  assert.equal(auditResult.ok, true)
  assert.match(auditResult.message ?? "", /home_extension\.invoke\.denied lookup denied/)
  assert.match(auditResult.message ?? "", /actor: home=alice caller=bob agent=agent-1 lease=lease-1 worker=worker-1 run=run-1/)
  assert.match(auditResult.message ?? "", /tool: script:lookup as=lookup safety=read timeout=30s hash=hash-tool-1/)
  assert.match(auditResult.message ?? "", /invocation: id=invoke-1 call=call-1 attempt=2 idempotency=idem-1/)
  assert.match(auditResult.message ?? "", /result: ok=false bytes=0 duration=24ms/)
  assert.match(auditResult.message ?? "", /error: worker mismatch/)
  assert.match(auditResult.message ?? "", /next: run \/extension sync-status agent-1; use \/extension sync-retry agent-1 after the worker\/provider run is current/)
  assert.doesNotMatch(auditResult.message ?? "", /not rendered/)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    { ListAgents: { session_id: "session-1" } },
    { SyncRemoteExtensionManifest: { agent_ref: "agent-1" } },
    { ListHomeExtensionAudit: { agent_ref: "agent-1", limit: 1 } },
  ])
})

test("executeShellCommand treats remote skill snapshots as passive for sync diagnostics", async () => {
  const agent = makeAgent({
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
    extension_grants: [{ kind: "skill", name: "qa" }],
  })
  const fake = fakeClient(() => ({ AgentsListed: { agents: [agent] } }))
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", agentId: "agent-1" })

  const grantsResult = await executeShellCommand(parseShellCommand("extension grants skill"), context, { client: fake.client })
  const statusResult = await executeShellCommand(parseShellCommand("extension sync-status agent-1"), context, { client: fake.client })

  assert.equal(grantsResult.ok, true)
  assert.match(grantsResult.message ?? "", /agent-1 skill grants:\n- qa \(skills snapshot\)/)
  assert.doesNotMatch(grantsResult.message ?? "", /remote extension sync:/)
  assert.equal(statusResult.ok, true)
  assert.match(statusResult.message ?? "", /agent-1 has no active home-proxy tools; skill grants are passive snapshots and no home-proxy manifest is projected\./)
  assert.doesNotMatch(statusResult.message ?? "", /manifest pending/)
})

test("executeShellCommand makes pending remote extension sync retryable", async () => {
  const agent = makeAgent({
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
    remote_extension_manifest_sync: {
      state: "pending",
      manifest_hash: "pending-hash",
    },
    extension_grants: [{ kind: "script", name: "lookup", environment: "py" }],
  })
  const fake = fakeClient(() => ({ AgentsListed: { agents: [agent] } }))
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", agentId: "agent-1" })
  const statusResult = await executeShellCommand(parseShellCommand("extension sync-status agent-1"), context, { client: fake.client })

  assert.equal(statusResult.ok, true)
  assert.match(statusResult.message ?? "", /agent-1 remote extension sync: pending/)
  assert.match(statusResult.message ?? "", /manifest hash: pending-hash/)
  assert.match(statusResult.message ?? "", /next: wait for the worker manifest update; run \/extension sync-status agent-1; run \/machine kernels machine-1 if it does not settle; use \/extension sync-retry agent-1 after worker connectivity is healthy/)
})

test("executeShellCommand manages script environments and script extensions", async () => {
  const environment = { name: "py", runtime: { type: "python", python: "/usr/bin/python3" } }
  const script = {
    name: "lookup",
    runtime: "python",
    path: "/repo/lookup.py",
    description: "Lookup a record.",
    input_schema: { type: "object", properties: { id: { type: "string" } }, required: ["id"] },
    definition_hash: "hash",
  }
  const agent = makeAgent({ extension_grants: [{ kind: "script", name: "lookup", environment: "py" }] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("RegisterEnvironment" in request) return { EnvironmentRegistered: { environment } }
        if ("RegisterScript" in request) return { ScriptRegistered: { script } }
        if ("ValidateScript" in request) return { ScriptValidated: { script } }
        if ("GrantAgentExtension" in request) return { AgentExtensionGranted: { agent } }
        if ("ListAgents" in request) return { AgentsListed: { agents: [agent] } }
        if ("ListScripts" in request) return { ScriptsListed: { scripts: [script] } }
        return { EnvironmentsListed: { environments: [environment] } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", agentId: "agent-1" })
  const envResult = await executeShellCommand(parseShellCommand("env register py --python /usr/bin/python3"), context, { client: fake.client })
  const validateResult = await executeShellCommand(parseShellCommand("script validate /repo/lookup.py --env py --name lookup"), context, { client: fake.client })
  const registerResult = await executeShellCommand(parseShellCommand("script register /repo/lookup.py --env py --name lookup"), context, { client: fake.client })
  const grantResult = await executeShellCommand(parseShellCommand("extension grant script agent-1 lookup --env py"), context, { client: fake.client })
  const listScriptsResult = await executeShellCommand(parseShellCommand("script list"), context, { client: fake.client })
  const listEnvsResult = await executeShellCommand(parseShellCommand("env list"), context, { client: fake.client })

  assert.equal(envResult.ok, true)
  assert.equal(validateResult.ok, true)
  assert.equal(registerResult.ok, true)
  assert.equal(grantResult.ok, true)
  assert.match(listScriptsResult.message ?? "", /lookup \[python\]/)
  assert.match(listEnvsResult.message ?? "", /py \[python\]/)
  assert.deepEqual(requests, [
    { RegisterEnvironment: { workspace_id: "/repo", config: environment } },
    { ValidateScript: { workspace_id: "/repo", source_path: "/repo/lookup.py", environment: "py", name: "lookup" } },
    { RegisterScript: { workspace_id: "/repo", source_path: "/repo/lookup.py", environment: "py", name: "lookup" } },
    { ListAgents: { session_id: "session-1" } },
    { GrantAgentExtension: { workspace_id: "/repo", agent_ref: "agent-1", kind: "script", name: "lookup", environment: "py" } },
    { ListScripts: { workspace_id: "/repo" } },
    { ListEnvironments: { workspace_id: "/repo" } },
  ])
})

test("executeShellCommand installs and uninstalls skills", async () => {
  const skill: ArrobaSkillMetadata = {
    name: "qa",
    description: "QA checks",
    short_description: "QA",
    path: "/skills/qa",
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InstallSkill" in request) {
          return { SkillInstalled: { skill } }
        }
        return { SkillUninstalled: { skill } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const installResult = await executeShellCommand(parseShellCommand("skill install /tmp/skills/qa"), context, { client: fake.client })
  const uninstallResult = await executeShellCommand(parseShellCommand("skill uninstall qa"), context, { client: fake.client })
  assert.equal(installResult.ok, true)
  assert.match(installResult.message ?? "", /installed skill qa/)
  assert.equal(uninstallResult.ok, true)
  assert.match(uninstallResult.message ?? "", /uninstalled skill qa/)
  assert.deepEqual(requests, [
    { InstallSkill: { workspace_id: "/repo", source_path: "/tmp/skills/qa" } },
    { UninstallSkill: { workspace_id: "/repo", name: "qa" } },
  ])
})
