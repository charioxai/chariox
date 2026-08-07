import assert from "node:assert/strict"
import test from "node:test"

import {
  assertNativeProviderAgentSelections,
  arrobaAttachmentImagePng,
  attachedImagePrompt,
  attachSubscribedTerminalClient,
  baselinePrompt,
  nativeProviderLaunchArgs,
  nativeProviderSelectedModel,
  nativeAttachmentImagePng,
  permissionInteractionForAlias,
} from "./live-remote-native-tui-drill-scenario.mjs"

test("remote native drills launch the explicitly requested provider models", () => {
  const options = {
    providerModels: {
      codex: "gpt-5.6-luna",
      claude: "sonnet",
      opencode: "opencode/kimi-k2.7-code",
    },
    codexEffort: "medium",
  }

  assert.deepEqual(nativeProviderLaunchArgs("codex", options), [
    "--model", "gpt-5.6-luna", "--effort", "medium", "--server-in-kernel",
  ])
  assert.deepEqual(nativeProviderLaunchArgs("opencode", options), [
    "--model", "opencode/kimi-k2.7-code", "--server-in-kernel",
  ])
  assert.deepEqual(nativeProviderLaunchArgs("claude", options), ["--model", "sonnet"])
  assert.equal(nativeProviderSelectedModel("codex", options), "gpt-5.6-luna")
  assert.equal(nativeProviderSelectedModel("opencode", options), "opencode/kimi-k2.7-code")
  assert.equal(nativeProviderSelectedModel("claude", options), "sonnet")
})

test("remote native drills verify and report actual kernel agent selections", () => {
  const options = {
    providerModels: {
      codex: "gpt-5.6-luna",
      claude: "sonnet",
      opencode: "opencode/kimi-k2.7-code",
    },
    codexEffort: "medium",
  }
  const agents = [
    { id: "agent-a", alias: "cdx-remote-a", provider: "codex", model: "gpt-5.6-luna", effort: "medium" },
    { id: "agent-b", alias: "cdx-remote-b", provider: "codex", model: "gpt-5.6-luna", effort: "medium" },
  ]

  assert.deepEqual(assertNativeProviderAgentSelections("codex", options, agents), agents)
  assert.deepEqual(assertNativeProviderAgentSelections("claude", options, [
    { id: "agent-c", alias: "cc-remote-a", provider: "claude", model: "claude/claude-sonnet-5", effort: "low" },
  ]), [
    { id: "agent-c", alias: "cc-remote-a", provider: "claude", model: "claude/claude-sonnet-5", effort: "low" },
  ])
  assert.deepEqual(assertNativeProviderAgentSelections("opencode", options, [
    { id: "agent-d", alias: "oc-remote-a", provider: "opencode", model: "opencode/kimi-k2.7-code", effort: null },
  ]), [
    { id: "agent-d", alias: "oc-remote-a", provider: "opencode", model: "opencode/kimi-k2.7-code", effort: null },
  ])
  assert.throws(
    () => assertNativeProviderAgentSelections("codex", options, [
      { ...agents[0], model: "default" },
      agents[1],
    ]),
    /expected codex\/gpt-5\.6-luna\/medium/,
  )
  assert.throws(
    () => assertNativeProviderAgentSelections("claude", options, [
      { id: "agent-c", alias: "cc-remote-a", provider: "claude", model: "claude/claude-opus-4-6", effort: "low" },
    ]),
    /expected claude\/sonnet\/low/,
  )
})

test("remote Claude drills use natural factual prompts instead of protocol sentinels", () => {
  const markers = {
    nativeA: "Mercury is the closest planet to the Sun",
    nativeB: "Jupiter is the largest planet in the Solar System",
    arrobaA: "Canberra is the capital of Australia",
    arrobaB: "Ottawa is the capital of Canada",
  }

  for (const promptKey of Object.keys(markers)) {
    const prompt = baselinePrompt("claude", promptKey, markers)
    assert.match(prompt, /Please answer this (astronomy|geography) question/)
    assert.match(prompt, new RegExp(markers[promptKey]))
    assert.doesNotMatch(prompt, /reply with exactly|CLAUDE[A-Z]+/i)
  }
  assert.equal(
    baselinePrompt("codex", "nativeA", { nativeA: "CODEXCHARLIE" }),
    "Reply with exactly CODEXCHARLIE and nothing else.",
  )
})

test("remote Claude attachment fixtures are visible images with natural prompts", () => {
  for (const fixture of [nativeAttachmentImagePng, arrobaAttachmentImagePng]) {
    assert.equal(fixture.readUInt32BE(16), 200)
    assert.equal(fixture.readUInt32BE(20), 200)
  }
  assert.equal(nativeAttachmentImagePng.equals(arrobaAttachmentImagePng), false)

  const prompt = attachedImagePrompt("dominant color is red", "claude")
  assert.match(prompt, /inspect the attached image/i)
  assert.match(prompt, /dominant color is red/)
  assert.doesNotMatch(prompt, /reply with exactly|CLAUDE[A-Z]+/i)
})

test("remote native drill subscribes its terminal attachment before using it", async () => {
  const calls = []
  const client = {
    async send(request) {
      calls.push({ operation: "send", request })
      return {
        SessionAttached: {
          attachment: { id: "attachment-observer" },
        },
      }
    },
    async subscribeToKernelEvents(sessionId, attachmentId) {
      calls.push({ operation: "subscribe", sessionId, attachmentId })
    },
  }

  const attachment = await attachSubscribedTerminalClient(
    client,
    "session-remote",
    "remote-native-drill",
  )

  assert.equal(attachment.id, "attachment-observer")
  assert.deepEqual(calls, [
    {
      operation: "send",
      request: {
        AttachToSession: {
          session_id: "session-remote",
          client_id: "remote-native-drill",
          capability_level: "FullTerminal",
        },
      },
    },
    {
      operation: "subscribe",
      sessionId: "session-remote",
      attachmentId: "attachment-observer",
    },
  ])
})

test("remote native drill finds permission interactions for the requested agent only", () => {
  const snapshot = {
    session: {
      agents: [
        { id: "agent-a", alias: "worker-a" },
        { id: "agent-b", alias: "worker-b" },
      ],
    },
    interactions: [
      { id: "request-a", agentId: "agent-a", kind: "user_request" },
      { id: "permission-b", agentId: "agent-b", kind: "permission" },
      { id: "permission-a", agentId: "agent-a", kind: "permission" },
    ],
  }

  assert.deepEqual(permissionInteractionForAlias(snapshot, "worker-a"), {
    snapshot,
    agent: snapshot.session.agents[0],
    interaction: snapshot.interactions[2],
  })
  assert.equal(permissionInteractionForAlias(snapshot, "missing"), null)
})
