import assert from "node:assert/strict"
import test from "node:test"
import { TextareaRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import { createFocusedInteractionChoiceController } from "./focused-interaction-choice-controller.js"

// The prompt textarea routes its key presses to the focused interaction (its
// onKeyDown), and pastes reach the interaction through the renderer's global
// paste listener (usePaste), as in cli-input-routing-composition.ts. These
// run OpenTUI's own terminal parser over the bytes a terminal sends.
for (const kittyKeyboard of [false, true]) {
  test(`a vault passphrase typed and pasted in the TUI keeps its case (kitty keyboard ${kittyKeyboard})`, async () => {
    const harness = await createTestRenderer({ width: 80, height: 8, useThread: false, kittyKeyboard })
    const replies = new Map<string, string>()
    const editing = new Set<string>(["vault-unlock"])
    const answers: Array<string | null> = []
    const controller = createFocusedInteractionChoiceController({
      getFocusedInteraction: () => passphraseInteraction,
      isAttached: () => true,
      getSessionId: () => "session-1",
      getSelectedIndex: () => 1,
      setSelectedIndex: () => {},
      getCustomReply: (id) => replies.get(id) ?? "",
      setCustomReply: (id, reply) => { replies.set(id, reply) },
      clearCustomReply: (id) => { replies.delete(id) },
      isCustomEditing: (id) => editing.has(id),
      setCustomEditing: (id, value) => { if (value) editing.add(id); else editing.delete(id) },
      renderAgentInteractions: () => {},
      applyResponseLayout: () => {},
      respondToInteraction: async (_session, _interaction, _choice, reply) => {
        answers.push(reply)
        return session
      },
      applySessionState: () => {},
      flashFooter: () => {},
    })
    const prompt = new TextareaRenderable(harness.renderer, {
      width: 40,
      height: 1,
      onKeyDown: (key) => { controller.handleKey(key) },
    })
    const paste = (event: Parameters<typeof controller.handlePaste>[0]) => { controller.handlePaste(event) }
    harness.renderer.root.add(prompt)
    harness.renderer.keyInput.on("paste", paste)
    const send = (bytes: string) => { harness.renderer.stdin.emit("data", Buffer.from(bytes)) }
    try {
      prompt.focus()
      if (kittyKeyboard) {
        // Shift+c without associated text, Shift+1 reporting its shifted
        // key, and keys with associated text (CSI code;mods;text u).
        send("\u001b[99;2u")
        send("\u001b[111u")
        send("\u001b[49:33;2u")
        send("\u001b[64;1;64u")
        send("\u001b[32u")
        send("\u001b[233;1;233u")
      } else {
        await harness.mockInput.typeText("Co!@ é")
      }
      send("\u{1D11E}")
      await harness.mockInput.pasteBracketedText("Pa$te Ünï\n")

      assert.equal(prompt.plainText, "", "no secret text reaches the prompt")
      assert.equal(replies.get("vault-unlock"), "Co!@ é\u{1D11E}Pa$te Ünï")

      send("\r")
      await new Promise((resolve) => setTimeout(resolve, 0))
      assert.deepEqual(answers, ["Co!@ é\u{1D11E}Pa$te Ünï"])
      assert.equal(prompt.plainText, "")
    } finally {
      harness.renderer.keyInput.off("paste", paste)
      harness.renderer.destroy()
    }
  })
}

const passphraseInteraction: RuntimeInteraction = {
  id: "vault-unlock",
  agent_id: "agent-1",
  kind: "choice",
  level: "critical",
  title: "Unlock Chariox Vault",
  message: "Enter the vault passphrase.",
  choices: [{ id: "cancel", label: "Cancel", reply: "cancel" }],
  custom_choice: {
    id: "passphrase",
    label: "Vault passphrase",
    placeholder: "Passphrase",
    min_length: 1,
    max_length: 512,
    input_kind: "secret",
  },
  timeout_sec: 300,
  default_on_timeout: "cancel",
  requested_at_ms: 1,
}

const session = {
  id: "session-1",
  project_id: "project-default",
  workspace_id: "/workspace",
  worktree_id: "/workspace/tree",
  created_at_ms: 1,
  status: "Created",
  active_provider_run_id: null,
  attachment_ids: [],
  active_prompt: null,
  queued_prompts: [],
  focused_agent_id: null,
  max_agents: 1,
  agents: [],
  config_state: { version: 1, values: {} },
} as RuntimeSession
