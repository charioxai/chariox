import assert from "node:assert/strict"
import test from "node:test"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { handoffActionLabel, handoffChangeLines, handoffOutcomeText, respondToHandoffRequest, type RuntimeHandoff } from "./owner-handoff.js"
import { readFileSync } from "node:fs"

const handoff: RuntimeHandoff = {
  kind: "click", reason: "model_refusal", agent_id: "agent", task_id: "task", obligation_id: "obligation",
  explanation: "Save the firewall rules", expires_at_ms: 1,
  target: { tab_id: "tab", generation: 1, document_id: "doc", node_ref: "backend:7", origin: "https://fw.test", path: "/rules", label: "Save" },
  change: [{ op: "keep", text: "allow tcp 22" }, { op: "add", text: "allow tcp 443" }],
}

test("MP-08/MP-11 A07: protocol 477 hand-off answers use the dedicated protected request", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 477)
  assert.deepEqual(respondToHandoffRequest("room", "handoff-obligation", { kind: "enter_value", value: "fixture", save_to_vault_key: "login" }), {
    RespondToHandoff: { session_id: "room", interaction_id: "handoff-obligation", action: { kind: "enter_value", value: "fixture", save_to_vault_key: "login" } },
  })
  assert.deepEqual(respondToHandoffRequest("room", "handoff-obligation", { kind: "click" }), {
    RespondToHandoff: { session_id: "room", interaction_id: "handoff-obligation", action: { kind: "click" } },
  })
  const ipc = readFileSync(new URL("../src/ipc.ts", import.meta.url), "utf8")
  assert.match(ipc, /KERNEL_REQUESTS_RUN_AGAIN_ON_REPLAY = new Set\(\[[^\]]*"RespondToHandoff"/,
    "physical owner input is never resent after a reconnect")
})

test("MP-08/MP-11 A07: owner sees the bound action, the diff and only the safe outcome", () => {
  assert.equal(handoffActionLabel(handoff), "Click “Save”")
  assert.deepEqual(handoffChangeLines(handoff), ["  allow tcp 22", "+ allow tcp 443"])
  assert.equal(handoffOutcomeText({ handoff_id: "h", status: "failed", action: "click", reason_code: "target_changed" }), "Hand-off failed (target changed)")
  assert.equal(handoffOutcomeText({ handoff_id: "h", status: "completed", action: "enter_value", saved_to_vault: true }), "Hand-off completed; saved to Vault")
})
