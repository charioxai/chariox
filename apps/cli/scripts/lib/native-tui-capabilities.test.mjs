import assert from "node:assert/strict"
import { access, mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  cleanupNativeDrillCapabilities,
  installNativeDrillCapabilities,
} from "./native-tui-capabilities.mjs"

function fakeClient(requests) {
  return {
    async send(request) {
      requests.push(request)
      if (request.InstallSkill) {
        return {
          SkillInstalled: {
            skill: { name: path.basename(request.InstallSkill.source_path) },
          },
        }
      }
      return { McpServerInstalled: {} }
    },
  }
}

test("installs matching MCP and skill capabilities on a relay-addressed Hetzner worker", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-native-capabilities-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const workspace = path.join(root, "workspace")
  const scenarioRoot = path.join(root, "scenario")
  const homeRequests = []
  const workerRequests = []
  const synced = []

  const installed = await installNativeDrillCapabilities({
    homeClient: fakeClient(homeRequests),
    workerKernelUrl: null,
    workerClient: fakeClient(workerRequests),
    syncWorkerCapabilityFiles: async (files) => {
      await access(files.mcpServerPath)
      await access(files.skillSource)
      synced.push(files)
    },
    provider: "codex",
    scenarioRoot,
    workspace,
    options: {
      includeMcpSkills: true,
      standardHomeWorker: true,
      hetznerWorker: true,
    },
    markers: {
      nativeSkill: "NATIVE_SKILL_MARKER",
      arrobaSkill: "ARROBA_SKILL_MARKER",
    },
  })

  assert.equal(synced.length, 1)
  assert.equal(homeRequests.length, 2)
  assert.equal(workerRequests.length, 2)
  assert.deepEqual(workerRequests, homeRequests)
  assert.equal(
    workerRequests[0].InstallMcpServer.config.transport.command,
    "node",
  )
  assert.equal(
    workerRequests[0].InstallMcpServer.config.transport.args[0],
    synced[0].mcpServerPath,
  )
  assert.equal(workerRequests[1].InstallSkill.source_path, synced[0].skillSource)

  await cleanupNativeDrillCapabilities(workspace, installed)
})
