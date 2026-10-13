import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable, TextareaRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { providerLoginView } from "@chariox/kernel-client/provider-login-projection"
import type { RuntimeInteraction } from "./cli-types.js"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { createProviderAccountRecoveryController } from "./provider-account-recovery-controller.js"
import { createProviderLoginPopupComposition } from "./provider-login-popup-composition.js"
import type { ProviderLoginInteractionController } from "./provider-login-interaction-controller.js"
import { routeRawPastes } from "./raw-paste-routing.js"

for (const field of ["provider-response", "passphrase", null]) {
  test(`MP-08/MP-11 login popup consumes paste with Cancel selected (${field ?? "no input"})`, async () => {
    const h = await createTestRenderer({ width: 100, height: 24, useThread: false })
    const choices = createInteractionChoiceStoreController()
    const flashes: string[] = []
    let current: RuntimeInteraction | null = {
      id: "login", agent_id: "provider-account:claude:fresh", kind: "choice", level: "warning", message: "Sign in", requested_at_ms: 42,
      choices: [{ id: "cancel", label: "Cancel", reply: "cancel" }],
      custom_choice: field ? { id: field, label: "Masked input", input_kind: "secret" } : null,
    }
    const login = { sessionlessInteraction: () => current, stripState: () => null } as unknown as ProviderLoginInteractionController
    const popup = createProviderLoginPopupComposition({ renderer: h.renderer, attached: () => false, login, choices,
      send: async () => { assert.fail("paste must not submit a reply") },
      flashFooter: message => { flashes.push(message) }, render: () => {} })
    const prompt = new TextareaRenderable(h.renderer, { width: 60, height: 1, onKeyDown: key => { popup.handleKey(key) } })
    h.renderer.root.add(prompt)
    const stopPastes = routeRawPastes(h.renderer.keyInput, popup.handlePaste)
    try {
      prompt.focus()
      assert.equal(popup.active(), true)
      if (field) {
        await h.mockInput.pressKeys(["ESCAPE"])
        await new Promise(resolve => setTimeout(resolve, 50))
        await h.mockInput.pressKeys(["ARROW_UP"])
      }
      assert.equal(choices.isCustomEditing("login"), false)
      assert.equal(choices.getSelectedIndex("login") ?? 0, 0)
      assert.equal(prompt.focused, true, "waiting-room prompt stays focused behind the popup")
      for (const text of ["SensitiveCode Ω", "Vault passphrase", "Ab\u001b[31mCd", "two\nlines"]) {
        await h.mockInput.pasteBracketedText(text)
        assert.equal(prompt.plainText, "", "popup must prevent any paste from reaching the prompt")
        assert.equal(choices.customReply("login"), "", "Cancel must not collect a secret")
      }
      assert.equal(flashes.length, 4, "rejected pastes explain how to focus the input")
      current = null
      await h.mockInput.pasteBracketedText("ordinary prompt")
      assert.equal(prompt.plainText, "ordinary prompt", "inactive popup leaves normal pastes alone")
    } finally { stopPastes(); popup.dispose(); h.renderer.destroy() }
  })
}

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
