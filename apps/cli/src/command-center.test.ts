import assert from "node:assert/strict"
import test from "node:test"

import { buildCommandCenterItems } from "./command-center.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"
import { workflowRegistrySuggestionEntriesFromResponse } from "./workflow-registry-command-center-entries.js"

test("buildCommandCenterItems shows root slash commands", () => {
  const items = buildCommandCenterItems("/", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "group" && item.label === "/provider"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/opencode"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/model"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/variant"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/view"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/config"), true)
  assert.equal(items.some((item) => item.kind === "command" && item.label === "/exit"), true)
  assert.equal(items.find((item) => item.kind === "group" && item.label === "/extension")?.description, "Grant, revoke, and inspect worker-local, home-proxy, and skill snapshot extensions (6)")
})

test("buildCommandCenterItems includes config subcommands", () => {
  const items = buildCommandCenterItems("/config", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "command" && item.value === "/config show"), true)
  assert.equal(items.some((item) => item.kind === "command" && item.value === "/config path"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.value === "/config workspace-live-sync "), true)
  const configSyncItems = buildCommandCenterItems("/config workspace-live-sync", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })
  assert.equal(configSyncItems.find((item) => item.value === "/config workspace-live-sync off")?.description, "Explicitly default new sessions to live sync off; other repositories stay unrestricted")
  assert.equal(configSyncItems.find((item) => item.value === "/config workspace-live-sync managed")?.description, "Default new sessions to managed sync when provider hosts support write fencing")
  assert.equal(configSyncItems.find((item) => item.value === "/config workspace-live-sync tracked")?.description, "Default new sessions to tracked turn-end sync; works on workers without managed write fencing")
})

test("buildCommandCenterItems includes kernel remote runtime diagnostics", () => {
  const items = buildCommandCenterItems("/kernel", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.find((item) => item.value === "/kernel health")?.description, "Show runtime health, remote readiness, and invariants")
  assert.equal(items.find((item) => item.value === "/kernel remote-runtime")?.description, "Show home authority, worker runs, slices, home-proxy tools, and live sync readiness")
})

test("buildCommandCenterItems describes home-proxy extension sync recovery", () => {
  const items = buildCommandCenterItems("/extension", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.find((item) => item.value === "/extension sync-status ")?.description, "Show home-authoritative manifest sync, worker projection, and recovery for an agent")
  assert.equal(items.find((item) => item.value === "/extension sync-retry ")?.description, "Retry worker projection of a home-authoritative extension manifest")
})

test("buildCommandCenterItems finds runtime recovery commands by user-facing terms", () => {
  const context = {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode" as const,
    focusedProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  }

  assert.equal(buildCommandCenterItems("/run missing", context).some((item) => item.value === "/kernel "), true)

  const staleManifest = new Set(buildCommandCenterItems("/extension manifest stale", context).map((item) => item.value))
  assert.equal(staleManifest.has("/extension sync-status "), true)
  assert.equal(staleManifest.has("/extension sync-retry "), true)

  const revokePending = new Set(buildCommandCenterItems("/extension sync revoke pending", context).map((item) => item.value))
  assert.equal(revokePending.has("/extension sync-status "), true)
  assert.equal(revokePending.has("/extension sync-retry "), true)

  const sliceMissing = new Set(buildCommandCenterItems("/slice missing", context).map((item) => item.value))
  assert.equal(sliceMissing.has("/slice doctor "), true)
  assert.equal(sliceMissing.has("/slice auth login "), true)

  const workspaceCollision = new Set(buildCommandCenterItems("/workspace sync collision", context).map((item) => item.value))
  assert.equal(workspaceCollision.has("/workspace sync conflicts"), true)

  const ignoreRules = new Set(buildCommandCenterItems("/workspace sync arrobaignore", context).map((item) => item.value))
  assert.equal(ignoreRules.has("/workspace sync ignore"), true)

  const agentPlacement = new Set(buildCommandCenterItems("/agent where does this run", context).map((item) => item.value))
  assert.equal(agentPlacement.has("/agent inspect "), true)

  const agentWorktree = new Set(buildCommandCenterItems("/agent worktree", context).map((item) => item.value))
  assert.equal(agentWorktree.has("/agent inspect "), true)
  assert.equal(agentWorktree.has("/agent list"), true)

  const workerReadiness = new Set(buildCommandCenterItems("/machine worker readiness", context).map((item) => item.value))
  assert.equal(workerReadiness.has("/machine kernels "), true)

  const machineProviderAccount = new Set(buildCommandCenterItems("/machine provider account missing", context).map((item) => item.value))
  assert.equal(machineProviderAccount.has("/machine kernels "), true)

  const sliceProviderAccount = new Set(buildCommandCenterItems("/slice provider account missing", context).map((item) => item.value))
  assert.equal(sliceProviderAccount.has("/slice doctor "), true)
  assert.equal(sliceProviderAccount.has("/slice auth login "), true)

  const sliceDisplay = new Set(buildCommandCenterItems("/slice display", context).map((item) => item.value))
  assert.equal(sliceDisplay.has("/slice screen "), true)
})

test("buildCommandCenterItems explains slice spawn primitives", () => {
  const items = buildCommandCenterItems("/agent", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.find((item) => item.value === "/agent spawn ")?.description, "Spawn local, remote, or slice agents; use --slice off, new, new:headed, or an existing slice")
})

test("buildCommandCenterItems surfaces local provider catalog fallback in selection rows", () => {
  const context = {
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode" as const,
    focusedProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  }

  assert.match(buildCommandCenterItems("/provider codex", context)[0]?.description ?? "", /local provider list/)
  assert.match(buildCommandCenterItems("/model gpt", context)[0]?.description ?? "", /local provider list/)
})

test("buildCommandCenterItems includes workspace live sync subcommands", () => {
  const items = buildCommandCenterItems("/workspace sync", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })
  const values = new Set(items.map((item) => item.value))

  assert.equal(values.has("/workspace sync status"), true)
  assert.equal(values.has("/workspace sync targets"), true)
  assert.equal(values.has("/workspace sync conflicts"), true)
  assert.equal(values.has("/workspace sync ignore"), true)
  assert.equal(values.has("/workspace sync audit"), true)
  assert.equal(values.has("/workspace sync off"), true)
  assert.equal(values.has("/workspace sync managed"), true)
  assert.equal(values.has("/workspace sync tracked"), true)
  assert.equal(values.has("/workspace sync default "), true)
  assert.equal(values.has("/workspace sync link "), true)
  assert.equal(values.has("/workspace sync enable managed"), false)
  assert.equal(values.has("/workspace sync enable tracked"), false)
  assert.equal(values.has("/workspace sync disable"), false)
  assert.equal(values.has("/workspace sync mode "), false)
  assert.equal(items.find((item) => item.value === "/workspace sync off")?.description, "Disable live sync for this session; other repositories stay unrestricted")
  assert.equal(items.find((item) => item.value === "/workspace sync audit")?.description, "Show workspace live sync mode audit events")
  assert.equal(items.find((item) => item.value === "/workspace sync managed")?.description, "Use managed sync for the selected workspace/worktree; other repositories stay unrestricted")
  assert.equal(items.find((item) => item.value === "/workspace sync tracked")?.description, "Use tracked sync for the selected workspace/worktree; other repositories stay unrestricted")
  assert.equal(items.find((item) => item.value === "/workspace sync default ")?.description, "Set default live sync for new sessions without changing the current session")
})

test("buildCommandCenterItems suggests registered workflow names and run endpoints", () => {
  const context = {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode" as const,
    focusedProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
    workflowRegistryEntries: [
      {
        name: "dev-team-small",
        sourceScope: "workspace" as const,
        sourceKind: "source_directory" as const,
        endpoints: ["entry", "review"],
        queues: ["urgent"],
      },
      {
        name: "prompt-chaining",
        sourceScope: "builtin" as const,
        sourceKind: "single_file" as const,
      },
    ],
  }

  const loadValues = new Set(buildCommandCenterItems("/workflow load ", context).map((item) => item.value))
  assert.equal(loadValues.has("/workflow load dev-team-small"), true)
  assert.equal(loadValues.has("/workflow load prompt-chaining"), true)
  assert.equal(buildCommandCenterItems("/workflow registry get dev", context)[0]?.description, "workspace · source_directory")

  const deleteValues = new Set(buildCommandCenterItems("/workflow registry delete ", context).map((item) => item.value))
  assert.equal(deleteValues.has("/workflow registry delete dev-team-small"), true)
  assert.equal(deleteValues.has("/workflow registry delete prompt-chaining"), false)

  const runValues = new Set(buildCommandCenterItems("/workflow run ", context).map((item) => item.value))
  assert.equal(runValues.has('/workflow run dev-team-small --endpoint entry --prompt "" '), true)
  assert.equal(runValues.has('/workflow run prompt-chaining --endpoint entry --prompt "" '), true)

  const endpointValues = new Set(buildCommandCenterItems("/workflow run dev-team-small --endpoint ", context).map((item) => item.value))
  assert.equal(endpointValues.has('/workflow run dev-team-small --endpoint entry --prompt "" '), true)
  assert.equal(endpointValues.has('/workflow run dev-team-small --endpoint review --prompt "" '), true)

  const queueValues = new Set(buildCommandCenterItems("/workflow run dev-team-small --queue ", context).map((item) => item.value))
  assert.equal(queueValues.has('/workflow run dev-team-small --endpoint entry --queue urgent --prompt "" '), true)
})

test("workflow registry command center entries read endpoint and queue summary metadata", () => {
  const entries = workflowRegistrySuggestionEntriesFromResponse({
    WorkflowRegistryListed: {
      entries: [{
        name: "dev-team-small",
        source_scope: "workspace",
        source_kind: "source_directory",
        source_path: "workflow.js",
        source_sha256: "source",
        source_bytes: 10,
        created_at_ms: 1,
        updated_at_ms: 2,
        validation: { ok: true },
        summary: {
          endpoints: ["entry", "review"],
          queues: ["urgent"],
          nodes: ["planner", "reviewer"],
          default_endpoint: "entry",
        },
      }],
    },
  })

  const context = {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode" as const,
    focusedProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
    workflowRegistryEntries: entries,
  }
  const endpointValues = new Set(buildCommandCenterItems("/workflow run dev-team-small --endpoint ", context).map((item) => item.value))
  const queueValues = new Set(buildCommandCenterItems("/workflow run dev-team-small --queue ", context).map((item) => item.value))

  assert.equal(endpointValues.has("/workflow run dev-team-small --endpoint entry --prompt \"\" "), true)
  assert.equal(endpointValues.has("/workflow run dev-team-small --endpoint review --prompt \"\" "), true)
  assert.equal(queueValues.has("/workflow run dev-team-small --endpoint entry --queue urgent --prompt \"\" "), true)
})

test("buildCommandCenterItems includes slice diagnostics and lifecycle commands", () => {
  const items = buildCommandCenterItems("/slice", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })
  const values = new Set(items.map((item) => item.value))

  assert.equal(values.has("/slice list"), true)
  assert.equal(values.has("/slice status "), true)
  assert.equal(values.has("/slice doctor "), true)
  assert.equal(values.has("/slice logs "), true)
  assert.equal(values.has("/slice audit "), true)
  assert.equal(values.has("/slice start "), true)
  assert.equal(values.has("/slice stop "), true)
  assert.equal(values.has("/slice delete "), true)
  assert.equal(values.has("/slice auth login "), true)
  assert.equal(items.find((item) => item.value === "/slice auth login ")?.description, "Start provider login inside the slice for a different account")
  assert.equal(items.find((item) => item.value === "/slice auth import ")?.description, "Copy this machine's provider credentials into the slice; credentials stay slice-scoped")
  assert.equal(items.find((item) => item.value === "/slice auth remove ")?.description, "Remove slice-local provider credentials and account summary")
  assert.equal(items.find((item) => item.value === "/slice auth alias ")?.description, "Set an Arroba display alias when the provider account label is unclear")
})

test("buildCommandCenterItems filters model options", () => {
  const items = buildCommandCenterItems("/model gpt", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "codex",
    focusedProvider: "codex",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.every((item) => item.kind === "model"), true)
  assert.equal(items.some((item) => item.value === "codex/gpt-5.4"), true)
  assert.equal(items.some((item) => item.value === "opencode/gpt-5.4"), false)
})

test("buildCommandCenterItems exposes Claude provider and models", () => {
  const providerItems = buildCommandCenterItems("/provider cla", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })
  assert.equal(providerItems.some((item) => item.kind === "provider" && item.value === "claude-headless"), true)
  assert.equal(providerItems.some((item) => item.kind === "provider" && item.value === "claude-p"), true)

  const modelItems = buildCommandCenterItems("/model son", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "claude-headless",
    focusedProvider: "claude-headless",
    currentModel: "claude-headless/claude-sonnet-4-6",
    currentVariant: "high",
  })
  assert.deepEqual(modelItems.map((item) => item.value), ["claude-headless/claude-sonnet-4-6"])

  const rootItems = buildCommandCenterItems("/", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "claude-headless",
    focusedProvider: "claude-headless",
    currentModel: "claude-headless/claude-sonnet-4-6",
    currentVariant: "high",
  })
  assert.equal(rootItems.some((item) => item.kind === "group" && item.label === "/claude"), false)
})


test("buildCommandCenterItems filters variant options", () => {
  const items = buildCommandCenterItems("/variant med", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.kind, "variant")
  assert.equal(items[0]?.value, "medium")
})

test("buildCommandCenterItems closes exact trailing-space commands", () => {
  const items = buildCommandCenterItems("/agent delete ", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.deepEqual(items, [])
})

test("buildCommandCenterItems shows the delete agent command", () => {
  const items = buildCommandCenterItems("/agent del", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "delete")
})

test("buildCommandCenterItems keeps the scoped parent visible for grouped commands", () => {
  const items = buildCommandCenterItems("/agent", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "/agent")
  assert.equal(items.some((item) => item.label === "spawn"), true)
  assert.equal(items.some((item) => item.label === "inspect"), true)
})

test("buildCommandCenterItems keeps the parent group visible while filtering scoped agent commands", () => {
  const items = buildCommandCenterItems("/agent sp", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "spawn")
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/agent"), true)
})

test("buildCommandCenterItems keeps provider options open after space", () => {
  const items = buildCommandCenterItems("/provider ", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "provider" && item.value === "opencode"), true)
  assert.equal(items.some((item) => item.kind === "provider" && item.value === "codex"), true)
})

test("buildCommandCenterItems keeps the parent group visible while filtering provider commands", () => {
  const items = buildCommandCenterItems("/provider st", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.label === "status"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/provider"), true)
})

test("buildCommandCenterItems shows multi-agent view options", () => {
  const items = buildCommandCenterItems("/view spl", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "split")
  assert.equal(items[0]?.value, "/view split")
})

test("buildCommandCenterItems includes workflow subcommands", () => {
  const items = buildCommandCenterItems("/workflow", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })
  const labels = new Set(items.map((item) => item.label))

  assert.equal(labels.has("list"), true)
  assert.equal(labels.has("show"), true)
  assert.equal(labels.has("new"), true)
  assert.equal(labels.has("node"), true)
  assert.equal(labels.has("edge"), true)
  assert.equal(labels.has("endpoint"), true)
  assert.equal(labels.has("/workflow"), true)
})

test("buildCommandCenterItems drills into workflow node subcommands", () => {
  const items = buildCommandCenterItems("/workflow node", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.label === "add"), true)
  assert.equal(items.some((item) => item.label === "add all"), true)
  assert.equal(items.some((item) => item.label === "remove"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "instructions"), true)
})

test("buildCommandCenterItems exposes workflow add node all shorthand", () => {
  const items = buildCommandCenterItems("/workflow add", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "add node all")
  assert.equal(items[0]?.value, "/workflow add node all")
})

test("buildCommandCenterItems exposes the focused provider namespace", () => {
  const items = buildCommandCenterItems("/codex", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "codex",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "group" && item.label === "/codex"), true)
})

test("buildCommandCenterItems lets root slash search surface parent groups from matching subcommands", () => {
  const items = buildCommandCenterItems("/reauth", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "group" && item.label === "/provider"), true)
})
