import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { providerLoginView } from "@chariox/kernel-client/provider-login-projection"
import type { RuntimeInteraction } from "./cli-types.js"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { createProviderAccountRecoveryController } from "./provider-account-recovery-controller.js"
import { createProviderLoginPopupComposition } from "./provider-login-popup-composition.js"
import type { ProviderLoginInteractionController } from "./provider-login-interaction-controller.js"

test("MP-08/MP-11 Provider Accounts shows the kernel link and masked input while detached, including Vault phases", async () => {
  const h = await createTestRenderer({ width: 140, height: 40, useThread: false })
  const box = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
  h.renderer.root.add(box)
  const projection = { kernel_id: "kernel", login: { provider: "claude", account_profile: "fresh", login_id: "login", login_kind: "terminal_setup_token",
    auth_url: "https://claude.com/cai/oauth/authorize?state=public-challenge" }, terminal_output_base64: "" }
  let current: RuntimeInteraction | null = { id: "login", agent_id: "provider-account:claude:fresh", kind: "choice", level: "warning",
    message: "Open the official link", requested_at_ms: 42, provider_login: projection,
    choices: [{ id: "cancel", label: "Cancel", reply: "cancel" }], custom_choice: { id: "provider-response", label: "Send response", input_kind: "secret" } }
  const login = {
    sessionlessInteraction: () => current,
    stripState: (interaction: RuntimeInteraction) => interaction.custom_choice?.id === "provider-response"
      ? { view: providerLoginView(projection, "provider-response", "fresh"), deadlineMs: null, checking: false, problem: null } : null,
    codeSent: () => true, openLink: async () => {}, copyLink: async () => {},
  } as unknown as ProviderLoginInteractionController
  const choices = createInteractionChoiceStoreController()
  const sent: unknown[] = []
  const popup = createProviderLoginPopupComposition({ renderer: h.renderer, attached: () => false, login, choices,
    send: async request => { assert.equal(choices.customReply("login"), ""); sent.push(request) },
    flashFooter: () => {}, render: () => accounts.render() })
  const accounts = createProviderAccountRecoveryController({ renderer: h.renderer, dimensions: () => ({ width: 140, height: 40 }), actions: () => [], loginSurface: popup })
  try {
    accounts.assignBox(box)
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /claude\.com\/cai\/oauth\/authorize/)
    assert.match(h.captureCharFrame(), /Paste the code below/)
    assert.equal(accounts.ownsInput(), true)
    assert.equal(popup.handlePaste({ text: "SensitiveCode Ω", preventDefault() {}, stopPropagation() {} }), true)
    await h.renderOnce()
    assert.doesNotMatch(h.captureCharFrame(), /SensitiveCode/)
    accounts.handleKey({ name: "return" })
    await Promise.resolve()
    assert.equal(sent.length, 1)
    current = { ...current!, title: "Unlock Chariox Vault", message: "Enter the Chariox Vault passphrase", custom_choice: { id: "passphrase", label: "Vault passphrase", input_kind: "secret" } }
    accounts.render()
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /Unlock Chariox Vault/)
    assert.match(h.captureCharFrame(), /Vault passphrase/)
    assert.doesNotMatch(h.captureCharFrame(), /Paste the code below/)
    assert.equal(choices.isCustomEditing("login"), true, "the kernel's next phase focuses its masked field")
    current = null
    accounts.render()
    assert.equal(accounts.ownsInput(), false)
    assert.equal(choices.customReply("login"), "")
  } finally { popup.dispose(); accounts.close(); h.renderer.destroy() }
})
