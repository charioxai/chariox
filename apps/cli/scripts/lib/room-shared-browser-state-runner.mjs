import assert from "node:assert/strict"
import { waitForRoomProviderSettlement } from "./live-room-provider-settlement.mjs"
import {
  assertRoomSharedBrowserStateText,
  roomSharedBrowserStateFields,
} from "./room-shared-browser-state-fixture.mjs"

const commandToolNames = new Set(["bash", "exec_command", "shell", "terminal"])

export async function runRoomSharedBrowserStatePhase(input) {
  const { client, requests, sessionId, providerAgentId, providerSessionId, fixtureUrl, generation, phase } = input
  assert.ok(["before-save", "after-restore"].includes(phase), "invalid Browser state evidence phase")
  assert.ok(typeof providerAgentId === "string" && providerAgentId.length > 0,
    "Browser state evidence requires the retained Room agent")
  assert.ok(typeof providerSessionId === "string" && providerSessionId.length > 0,
    "Browser state evidence requires the retained provider thread")
  assert.ok(typeof fixtureUrl === "string" && fixtureUrl.startsWith("http://"),
    "Browser state evidence requires the existing local fixture URL")
  assert.ok(/^[a-zA-Z0-9-]{1,96}$/.test(generation), "Browser state evidence requires its fixture generation")

  const prompt = roomSharedBrowserStatePrompt({ phase, fixtureUrl, generation })
  const attachment = unwrap(await client.send(requests.attachToSessionRequest(
    sessionId, `room-browser-state-${phase}`,
  )), "SessionAttached").attachment
  assert.ok(attachment?.id, "Browser state evidence attachment is missing")
  let promptId
  try {
    const submitted = unwrap(await client.send(requests.submitPromptRequest(
      sessionId, attachment.id, providerAgentId, prompt, [],
    )), "PromptSubmitted")
    promptId = (submitted.outcome?.Started ?? submitted.outcome?.Queued)?.prompt?.id
  } finally {
    await client.send(requests.detachFromSessionRequest(attachment.id))
  }
  assert.ok(typeof promptId === "string" && promptId.length > 0,
    "Browser state evidence prompt lacks a kernel prompt identity")

  const settlement = await waitForRoomProviderSettlement(input, providerAgentId, promptId)
  const outline = unwrap(await input.withTimeout(client.send(requests.getSessionHistoryOutlineRequest(
    sessionId, [providerAgentId], 2,
  )), 5_000, "Browser state provider evidence"), "SessionHistoryOutline")
  const turn = outline.agents?.find(agent => agent.agent_id === providerAgentId)?.turns
    ?.find(candidate => candidate.prompt_id === promptId)
  assert.ok(turn && turn.lifecycle === "completed" && turn.turn_id === settlement.turnId,
    "Browser state evidence must come from the exact completed provider turn")
  const records = await readProviderToolRecords({ turn, input, providerAgentId })
  const evidence = assertRoomSharedBrowserStatePhaseEvidence({
    records, phase, fixtureUrl, generation, expectedProviderSessionId: providerSessionId,
    actualProviderSessionId: turn.external_provider_session_id,
  })
  return {
    phase,
    agentId: providerAgentId,
    promptId,
    turnId: turn.turn_id,
    providerSessionId: turn.external_provider_session_id,
    browserMarkers: evidence.browserMarkers,
    rendererSandbox: evidence.rendererSandbox,
    graphicalProgram: evidence.graphicalProgram,
    toolCalls: evidence.toolCalls,
  }
}

export function roomSharedBrowserStatePrompt({ phase, fixtureUrl, generation }) {
  assert.ok(["before-save", "after-restore"].includes(phase), "invalid Browser state prompt phase")
  assert.ok(typeof fixtureUrl === "string" && fixtureUrl.startsWith("http://"), "invalid fixture URL")
  assert.ok(/^[a-zA-Z0-9-]{1,96}$/.test(generation), "invalid fixture generation")
  const programName = `chariox-room-state-${generation}`
  const programMarker = `ROOM_GRAPHICAL_PROGRAM_${generation}`
  const programPath = `"$HOME/.local/bin/${programName}"`
  const command = phase === "before-save"
    ? [
      "set -eu",
      `program=${programPath}`,
      'mkdir -p "$HOME/.local/bin"',
      `cat > "$program" <<'CHARIOX_ROOM_PROGRAM'\n#!/bin/sh\nexec /usr/bin/xmessage -timeout 45 -title "Chariox Browser state" "${programMarker}"\nCHARIOX_ROOM_PROGRAM`,
      'chmod 700 "$program"',
      'test -x "$program"',
      "printf '%s\\n' ROOM_GRAPHICAL_PROGRAM_INSTALLED",
      '"$program" >/dev/null 2>&1 &',
    ].join("\n")
    : [
      "set -eu",
      `program=${programPath}`,
      'test -x "$program"',
      "printf '%s\\n' ROOM_GRAPHICAL_PROGRAM_PRESENT_AFTER_RESTORE",
      '"$program" >/dev/null 2>&1 &',
    ].join("\n")
  const commandMarker = phase === "before-save"
    ? "ROOM_GRAPHICAL_PROGRAM_INSTALLED"
    : "ROOM_GRAPHICAL_PROGRAM_PRESENT_AFTER_RESTORE"

  return [
    "Use the existing slice-bound provider command tool for this Room agent; do not use host commands, fixture internals, Chariox meta commands, or another agent.",
    "Run exactly this command in the slice. It creates or checks only the unique executable under the slice user's home. Do not include browser cookies or credentials in command output.",
    "```sh", command, "```",
    `The command must print exactly ${commandMarker} after the file check. Run the graphical program in the background.`,
    `Use slice_find_text with query=${JSON.stringify(programMarker)} and verify that exact marker is visible on the shared desktop.`,
    "Then use slice_open_url with url='chrome://sandbox' and use slice_browser_text to inspect the actual Chromium sandbox status.",
    "Only report the renderer sandbox as active if both Namespace sandbox and Seccomp-BPF sandbox say Yes.",
    `Return to this same fixture tab with slice_open_url url=${JSON.stringify(fixtureUrl)}. Do not clear storage or re-create any state.`,
    "Use slice_browser_wait_for_text with text='ROOM_BROWSER_STATE cookie=PASS' and then call slice_browser_text once to read the completed fixture state report.",
    `The only acceptable state fields are ${roomSharedBrowserStateFields.join(", ")}, each PASS. The fixture must read its existing state in this phase; do not write markers.`,
    "Do not quote, copy, or disclose the cookie value or any credential. Report only the marker names, PASS/FAIL, and the provider tool outcomes.",
  ].join("\n")
}

export function assertRoomSharedBrowserStatePhaseEvidence(input) {
  const { records, phase, fixtureUrl, generation, expectedProviderSessionId, actualProviderSessionId } = input
  assert.equal(actualProviderSessionId, expectedProviderSessionId,
    "Browser state proof continued on a different provider thread")
  const tools = completedTools(records)
  const commandTools = tools.filter(tool => commandToolNames.has(normalizeToolName(tool.name)))
  assert.equal(commandTools.length, 1, "Browser state phase requires one completed slice provider command")
  const command = commandTools[0]
  const commandInput = stringify(command.input)
  const commandOutput = stringify(command.output)
  const programPath = `chariox-room-state-${generation}`
  const commandMarker = phase === "before-save"
    ? "ROOM_GRAPHICAL_PROGRAM_INSTALLED"
    : "ROOM_GRAPHICAL_PROGRAM_PRESENT_AFTER_RESTORE"
  assert.ok(commandInput.includes(programPath),
    "provider command did not reference the expected graphical program in the slice home")
  assert.ok(commandOutput.includes(commandMarker), "provider command output omitted the installed-program marker")
  if (phase === "before-save") {
    assert.ok(commandInput.includes("/usr/bin/xmessage") && commandInput.includes("cat >")
      && commandInput.includes("chmod 700"),
      "pre-save provider command did not create and mark the graphical program executable")
  } else {
    assert.ok(commandInput.includes("test -x") && !commandInput.includes("cat >")
      && !commandInput.includes("chmod "),
    "post-restore provider command must verify the saved program without recreating it")
  }

  const programMarker = `ROOM_GRAPHICAL_PROGRAM_${generation}`
  const visibleProgram = tools.filter(tool => hasToolSuffix(tool.name, "slice_find_text")
    && stringify(tool.output).includes(programMarker))
  assert.equal(visibleProgram.length, 1,
    "the installed graphical program marker must be visible through the Room Computer observer")

  const navigations = tools.filter(tool => hasToolSuffix(tool.name, "slice_open_url"))
  assert.equal(navigations.length, 2, "sandbox proof must inspect and leave one internal sandbox page")
  assert.ok(navigations.some(tool => stringify(tool.input).includes("chrome://sandbox")),
    "provider did not open Chromium's sandbox status page")
  assert.ok(navigations.some(tool => stringify(tool.input).includes(fixtureUrl)),
    "provider did not return to the original Browser fixture tab")
  const browserTexts = tools.filter(tool => hasToolSuffix(tool.name, "slice_browser_text"))
  assert.equal(browserTexts.length, 2, "provider must read sandbox and Browser state text once each")
  const sandboxText = browserTexts.map(tool => stringify(tool.output))
    .find(text => text.includes("Seccomp") && text.includes("sandbox"))
  assert.ok(sandboxText, "provider tool history omitted chrome://sandbox page text")
  assertRoomRendererSandboxText(sandboxText)
  const browserStateText = browserTexts.map(tool => stringify(tool.output))
    .find(text => text.includes("ROOM_BROWSER_STATE"))
  assert.ok(browserStateText, "provider tool history omitted local Browser state text")
  const browserMarkers = assertRoomSharedBrowserStateText(browserStateText)
  const waits = tools.filter(tool => hasToolSuffix(tool.name, "slice_browser_wait_for_text")
    && stringify(tool.input).includes("ROOM_BROWSER_STATE cookie=PASS"))
  assert.equal(waits.length, 1, "provider must wait for the state fixture through the Browser tool")

  return {
    browserMarkers,
    rendererSandbox: { namespace: "active", seccompBpf: "active" },
    graphicalProgram: { commandCompleted: true, visible: true, retained: phase === "after-restore" },
    toolCalls: {
      providerCommand: normalizeToolName(command.name),
      browserText: browserTexts.length,
      browserNavigations: navigations.length,
      computerTextObservation: visibleProgram.length,
    },
  }
}

export function assertRoomRendererSandboxText(text) {
  assert.equal(typeof text, "string", "renderer sandbox evidence must be text")
  assert.match(text, /namespace sandbox\s*:\s*yes\b/i,
    "Chromium namespace renderer sandbox was not proven active")
  assert.match(text, /seccomp[- ]bpf sandbox\s*:\s*yes\b/i,
    "Chromium Seccomp-BPF renderer sandbox was not proven active")
  return true
}

async function readProviderToolRecords({ turn, input, providerAgentId }) {
  const entries = [...(turn.entries ?? []), ...(turn.summary ? [turn.summary] : [])]
  const blobs = (turn.blobs ?? []).filter(blob => ["provider_tool", "provider_output"].includes(blob.kind))
  let totalChars = entries.reduce((sum, item) => sum + (typeof item?.entry?.text === "string" ? item.entry.text.length : 0), 0)
  assert.ok(entries.length <= 256 && totalChars <= 131072,
    "provider command evidence exceeded the bounded inline history inspection")
  for (const blob of blobs.slice(0, 16)) {
    assert.ok(Number.isSafeInteger(blob.total_chars) && blob.total_chars >= 0 && blob.total_chars <= 32768,
      "provider command evidence blob exceeded the bounded inspection size")
    assert.ok(totalChars + blob.total_chars <= 131072,
      "provider command evidence exceeded the bounded history inspection")
    const response = unwrap(await input.withTimeout(clientSend(input, input.requests.getSessionHistoryBlobContentRequest(
      input.sessionId, providerAgentId, blob.blob_id,
    )), 5_000, "Browser state provider tool evidence"), "SessionHistoryBlobContent")
    entries.push(...(response.entries ?? []))
    totalChars += blob.total_chars
  }
  assert.ok(blobs.length <= 16, "provider tool history exceeded the bounded Browser state evidence inspection")
  return uniqueProviderToolRecords(entries)
}

function uniqueProviderToolRecords(entries) {
  const positions = new Map()
  const records = []
  for (const item of entries.slice(0, 256)) {
    if (!["provider_tool", "provider_output"].includes(item?.entry?.kind)
      || typeof item.entry.text !== "string" || item.entry.text.length > 32768) continue
    let record
    try { record = JSON.parse(item.entry.text) } catch { continue }
    const name = record?.tool ?? record?.tool_name ?? record?.name
    if (typeof name !== "string") continue
    const callId = record.call_id ?? record.tool_call_id ?? record.id
    const key = callId
      ? `${name}:${callId}`
      : `${name}:${stringify(record.input ?? record.arguments)}:${stringify(record.output ?? record.result)}`
    const normalized = { name, status: record.status, input: record.input ?? record.arguments,
      output: record.output ?? record.result, sourceKind: item.entry.kind }
    const priorIndex = positions.get(key)
    if (priorIndex === undefined) {
      positions.set(key, records.length)
      records.push(normalized)
    } else {
      const prior = records[priorIndex]
      records[priorIndex] = {
        ...prior,
        status: normalized.status ?? prior.status,
        output: normalized.output ?? prior.output,
        sourceKind: normalized.sourceKind === "provider_output" ? normalized.sourceKind : prior.sourceKind,
      }
    }
  }
  return records
}

function completedTools(records) {
  return records.filter(record => record.sourceKind === "provider_output"
    || ["completed", "success", "succeeded", "ok"].includes(String(record.status ?? "").toLowerCase()))
}

function hasToolSuffix(name, suffix) {
  return normalizeToolName(name).endsWith(suffix)
}

function normalizeToolName(name) {
  return String(name).split("__").at(-1).split(".").at(-1).toLowerCase()
}

function stringify(value) {
  if (typeof value === "string") return value
  if (value === undefined) return ""
  try { return JSON.stringify(value) } catch { return String(value) }
}

function clientSend(input, request) {
  return input.client.send(request)
}

function unwrap(response, variant) {
  assert.ok(response && variant in response, `kernel did not return ${variant}`)
  return response[variant]
}
