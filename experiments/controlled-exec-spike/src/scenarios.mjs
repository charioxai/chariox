#!/usr/bin/env node
import assert from "node:assert/strict"
import { fileURLToPath } from "node:url"
import { mkdir, mkdtemp, rm, writeFile, readFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

import { ControlledExecHarness } from "./controlled-exec-harness.mjs"
import { FakeAgent } from "./fake-agent.mjs"
import { InteractionGateway } from "./interaction-gateway.mjs"
import { launchCodexServer } from "../../mcp-isolation-spike/src/codex-driver.mjs"
import { launchOpenCodeServer } from "../../mcp-isolation-spike/src/opencode-driver.mjs"

const spikeRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const mcpServerPath = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "controlled-exec-mcp-server.mjs")
const artifactsRoot = path.join(spikeRoot, "artifacts")

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, "-")
}

async function withTempDir(fn) {
  const dir = await mkdtemp(path.join(os.tmpdir(), "chariox-controlled-exec-"))
  try {
    return await fn(dir)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
}

async function scenarioBasicExecution() {
  return await withTempDir(async (cwd) => {
    const interactions = new InteractionGateway({ responses: ["yes", "yes", "yes"] })
    const harness = new ControlledExecHarness({ interactions })
    const agent = new FakeAgent({ agentId: "agent-a", harness, interactions, cwd })

    const pwd = await agent.execute("pwd", "limited", "turn-basic-1")
    const list = await agent.execute("mkdir -p outputs && ls", "limited", "turn-basic-2")
    const write = await agent.execute("echo hello > outputs/hello.txt", "limited", "turn-basic-3")
    const content = await readFile(path.join(cwd, "outputs/hello.txt"), "utf8")

    assert.equal(pwd.ok, true)
    assert.equal(list.ok, true)
    assert.equal(write.ok, true)
    assert.equal(content.trim(), "hello")

    return {
      name: "basic-execution",
      passed: true,
      details: {
        pwd: pwd.stdout.trim(),
        lsExitCode: list.exitCode,
        writeExitCode: write.exitCode,
      },
    }
  })
}

async function scenarioLimitedApproval() {
  return await withTempDir(async (cwd) => {
    const interactions = new InteractionGateway({ responses: ["no", "yes"] })
    const harness = new ControlledExecHarness({ interactions })
    const agent = new FakeAgent({ agentId: "agent-a", harness, interactions, cwd })

    const denied = await agent.execute("echo blocked > denied.txt", "limited", "turn-limited-1")
    const approved = await agent.execute("echo allowed > allowed.txt", "limited", "turn-limited-2")

    assert.equal(denied.allowed, false)
    assert.equal(approved.ok, true)
    assert.equal(interactions.history.length, 2)

    return {
      name: "limited-approval",
      passed: true,
      details: {
        deniedReason: denied.deniedReason,
        approvedExitCode: approved.exitCode,
        interactionCount: interactions.history.length,
      },
    }
  })
}

async function scenarioYoloOwnedDelete() {
  return await withTempDir(async (cwd) => {
    const interactions = new InteractionGateway()
    const harness = new ControlledExecHarness({ interactions })
    const agent = new FakeAgent({ agentId: "agent-a", harness, interactions, cwd })

    await writeFile(path.join(cwd, "preexisting.txt"), "keep\n", "utf8")
    await mkdir(path.join(cwd, "mixed"), { recursive: true })
    await writeFile(path.join(cwd, "mixed", "foreign.txt"), "foreign\n", "utf8")

    const createOwned = await agent.execute("mkdir -p owned && echo mine > owned/file.txt", "yolo", "turn-yolo-1")
    const deleteOwned = await agent.execute("rm -rf owned", "yolo", "turn-yolo-2")
    const deleteForeign = await agent.execute("rm -f preexisting.txt", "yolo", "turn-yolo-3")
    const createMixedDir = await agent.execute("mkdir -p mixed/owned-child && echo child > mixed/owned-child/owned.txt", "yolo", "turn-yolo-4")
    const deleteMixed = await agent.execute("rm -rf mixed", "yolo", "turn-yolo-5")

    assert.equal(createOwned.ok, true)
    assert.equal(deleteOwned.ok, true)
    assert.equal(deleteForeign.allowed, false)
    assert.equal(createMixedDir.ok, true)
    assert.equal(deleteMixed.allowed, false)

    return {
      name: "yolo-owned-delete",
      passed: true,
      details: {
        deleteForeignDenied: deleteForeign.deniedReason,
        deleteMixedDenied: deleteMixed.deniedReason,
      },
    }
  })
}

async function scenarioYoloRm() {
  return await withTempDir(async (cwd) => {
    const interactions = new InteractionGateway()
    const harness = new ControlledExecHarness({ interactions })
    const agent = new FakeAgent({ agentId: "agent-a", harness, interactions, cwd })
    await mkdir(path.join(cwd, "fixture"), { recursive: true })
    await writeFile(path.join(cwd, "fixture", "a.txt"), "a\n", "utf8")

    const result = await agent.execute("rm -rf fixture", "yolo+rm", "turn-yolo-rm-1")
    assert.equal(result.ok, true)
    return {
      name: "yolo-rm",
      passed: true,
      details: {
        exitCode: result.exitCode,
      },
    }
  })
}

async function scenarioMidTurnChoice() {
  return await withTempDir(async (cwd) => {
    const interactions = new InteractionGateway({ responses: [{ choiceId: "b" }, "yes"] })
    const harness = new ControlledExecHarness({ interactions })
    const agent = new FakeAgent({ agentId: "agent-a", harness, interactions, cwd })

    const choice = await agent.requestChoice({
      title: "Pick target",
      message: "Which file should I edit?",
      choices: ["a", "b", "c"],
      defaultChoice: "a",
      turnId: "turn-choice-1",
    })
    const result = await agent.execute(`echo chosen > ${choice.choiceId}.txt`, "limited", "turn-choice-1")
    const content = await readFile(path.join(cwd, "b.txt"), "utf8")

    assert.equal(choice.choiceId, "b")
    assert.equal(result.ok, true)
    assert.equal(content.trim(), "chosen")

    return {
      name: "mid-turn-choice",
      passed: true,
      details: {
        choiceId: choice.choiceId,
        interactionCount: interactions.history.length,
      },
    }
  })
}

async function scenarioBlockingPopupTimeoutDefault() {
  return await withTempDir(async (cwd) => {
    const interactions = new InteractionGateway()
    const agent = new FakeAgent({
      agentId: "agent-a",
      harness: new ControlledExecHarness({ interactions }),
      interactions,
      cwd,
    })

    const startedAt = Date.now()
    const result = await agent.requestPopup({
      title: "Need direction",
      message: "Pick a fallback.",
      level: "warning",
      choices: [
        { id: "skip", label: "Skip", reply: "Continue without user input." },
        { id: "retry", label: "Retry", reply: "Retry the step." },
      ],
      defaultChoice: "skip",
      timeoutSec: 1,
      turnId: "turn-popup-timeout-1",
    })
    const elapsedMs = Date.now() - startedAt

    assert.equal(result.status, "timed_out")
    assert.equal(result.choiceId, "skip")
    assert.equal(result.reply, "Continue without user input.")
    assert.equal(elapsedMs >= 900, true)

    return {
      name: "blocking-popup-timeout-default",
      passed: true,
      details: {
        status: result.status,
        choiceId: result.choiceId,
        elapsedMs,
      },
    }
  })
}

async function scenarioBlockingPopupExternalResolution() {
  return await withTempDir(async (cwd) => {
    const interactions = new InteractionGateway()
    const agent = new FakeAgent({
      agentId: "agent-a",
      harness: new ControlledExecHarness({ interactions }),
      interactions,
      cwd,
    })

    let observedInteractionId = null
    const requestPromise = agent.requestPopup({
      title: "Need answer",
      message: "Choose one.",
      level: "info",
      choices: [
        { id: "a", label: "A", reply: "Picked A." },
        { id: "b", label: "B", reply: "Picked B." },
      ],
      turnId: "turn-popup-external-1",
    })
    await new Promise((resolve) => setTimeout(resolve, 150))
    observedInteractionId = interactions.history.at(-1)?.interactionId ?? null
    assert.notEqual(observedInteractionId, null)
    setTimeout(() => {
      interactions.resolveInteraction(observedInteractionId, {
        choiceId: "b",
        reply: "Picked B.",
      })
    }, 250)

    const startedAt = Date.now()
    const result = await requestPromise
    const elapsedMs = Date.now() - startedAt

    assert.equal(result.status, "answered")
    assert.equal(result.choiceId, "b")
    assert.equal(result.reply, "Picked B.")
    assert.equal(elapsedMs >= 200, true)

    return {
      name: "blocking-popup-external-resolution",
      passed: true,
      details: {
        interactionId: observedInteractionId,
        elapsedMs,
      },
    }
  })
}

export async function runFakeScenarios() {
  const scenarios = [
    scenarioBasicExecution,
    scenarioLimitedApproval,
    scenarioYoloOwnedDelete,
    scenarioYoloRm,
    scenarioMidTurnChoice,
    scenarioBlockingPopupTimeoutDefault,
    scenarioBlockingPopupExternalResolution,
  ]
  const results = []
  for (const scenario of scenarios) {
    try {
      results.push(await scenario())
    } catch (error) {
      results.push({
        name: scenario.name,
        passed: false,
        error: String(error?.stack ?? error),
      })
    }
  }
  return {
    spike: "controlled-exec",
    mode: "fake",
    passed: results.every((result) => result.passed),
    results,
  }
}

export async function runProviderScenarios() {
  const cwd = path.join(artifactsRoot, `${timestamp()}-provider-drill`)
  await mkdir(cwd, { recursive: true })
  return await (async () => {
    const codexFile = path.join(cwd, "codex-choice.txt")
    const opencodeFile = path.join(cwd, "opencode-choice.txt")
    const sharedResponses = JSON.stringify(["yes", { choiceId: "beta" }, "yes"])
    const popupDelayMs = 1500
    const mcpConfig = {
      name: "controlled_exec",
      command: "node",
      args: [mcpServerPath, "--cwd", cwd, "--responses", sharedResponses, "--response-delay-ms", String(popupDelayMs)],
    }

    const results = []

    try {
      const codexRun = await launchCodexServer({
        agentId: "controlled-exec-codex",
        mcps: [mcpConfig],
        artifactDir: cwd,
      })
      try {
        const socket = await codexRun.connectInitialized()
        try {
          const started = await socket.threadStart({ cwd: spikeRoot, ephemeral: false, model: process.env.CODEX_SPIKE_MODEL || "gpt-5.4" })
          const threadId = started?.thread?.id
          if (!threadId) throw new Error("codex thread/start did not return a thread id")
          const startedAt = Date.now()
          const turn = await socket.turnStart(
            threadId,
            [
              "Use the MCP tools now.",
              "1. Call request_popup with title 'Pick target', message 'Pick one', level 'warning', timeout_sec 30, default_on_timeout 'alpha', and choices [{id:'alpha',label:'Alpha',reply:'User picked alpha.'},{id:'beta',label:'Beta',reply:'User picked beta.'}].",
              "2. Call controlled_exec with permission_mode 'limited' and command `printf 'codex:%s\\n' beta > codex-choice.txt`.",
              "3. Reply exactly CONTROLLED_EXEC_CODEX_OK.",
              "Do not just describe the tool call. Actually call the tools."
            ].join("\n"),
            { cwd: spikeRoot, model: process.env.CODEX_SPIKE_MODEL || "gpt-5.4", effort: "medium" },
          )
          await socket.waitForTurnCompleted(turn.turnId, 180_000)
          const elapsedMs = Date.now() - startedAt
          const transcript = socket.transcriptText()
          const fileText = await readFile(codexFile, "utf8").catch(() => "")
          await writeFile(path.join(cwd, "codex-transcript.txt"), transcript, "utf8")
          results.push({
            provider: "codex",
            passed: transcript.includes("CONTROLLED_EXEC_CODEX_OK") && fileText.trim() === "codex:beta" && elapsedMs >= popupDelayMs,
            transcript,
            fileText: fileText.trim(),
            elapsedMs,
          })
        } finally {
          await socket.close()
        }
      } finally {
        await codexRun.stop()
      }
    } catch (error) {
      results.push({ provider: "codex", passed: false, error: String(error?.stack ?? error) })
    }

    try {
      const openCodeRun = await launchOpenCodeServer({
        agentId: "controlled-exec-opencode",
        mcps: [mcpConfig],
        artifactDir: cwd,
      })
      try {
        const session = await openCodeRun.createSession({ directory: cwd, title: "controlled-exec-spike" })
        const sessionId = session?.id ?? session?.sessionID ?? session?.sessionId
        if (!sessionId) throw new Error(`OpenCode createSession returned no id: ${JSON.stringify(session)}`)
        const startedAt = Date.now()
        await openCodeRun.prompt(
          sessionId,
          [
            "Use the MCP tools now.",
            "1. Call request_popup with title 'Pick target', message 'Pick one', level 'warning', timeout_sec 30, default_on_timeout 'alpha', and choices [{id:'alpha',label:'Alpha',reply:'User picked alpha.'},{id:'beta',label:'Beta',reply:'User picked beta.'}].",
            "2. Call controlled_exec with permission_mode 'limited' and command `printf 'opencode:%s\\n' beta > opencode-choice.txt`.",
            "3. Reply exactly CONTROLLED_EXEC_OPENCODE_OK.",
            "Do not just describe the tool call. Actually call the tools."
          ].join("\n"),
          { directory: cwd, model: process.env.OPENCODE_SPIKE_MODEL || "opencode/gpt-5.4", variant: process.env.OPENCODE_SPIKE_VARIANT || "medium" },
        )
        const elapsedMs = Date.now() - startedAt
        const transcript = await openCodeRun.transcriptText(sessionId, { directory: cwd })
        const fileText = await readFile(opencodeFile, "utf8").catch(() => "")
        await writeFile(path.join(cwd, "opencode-transcript.txt"), transcript, "utf8")
        results.push({
          provider: "opencode",
          passed: transcript.includes("CONTROLLED_EXEC_OPENCODE_OK") && fileText.trim() === "opencode:beta" && elapsedMs >= popupDelayMs,
          transcript,
          fileText: fileText.trim(),
          elapsedMs,
        })
      } finally {
        await openCodeRun.stop()
      }
    } catch (error) {
      results.push({ provider: "opencode", passed: false, error: String(error?.stack ?? error) })
    }

    return {
      spike: "controlled-exec",
      mode: "providers",
      passed: results.every((result) => result.passed),
      artifactDir: cwd,
      results,
    }
  })()
}

async function main() {
  const mode = process.argv.includes("--mode=providers")
    ? "providers"
    : "fake"
  const result = mode === "providers"
    ? await runProviderScenarios()
    : await runFakeScenarios()
  console.log(JSON.stringify(result, null, 2))
  if (!result.passed && !result.deferred) {
    process.exitCode = 1
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  await main()
}
