import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { RuntimeInteraction } from "./cli-types.js"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"
import { createProviderLoginInteractionController } from "./provider-login-interaction-controller.js"

// MP-08/MP-11: the shape of the official Claude authorization URL.
const url = "https://claude.ai/oauth/authorize?code=true&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&response_type=code"
  + "&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference"
  + "&code_challenge=AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcdefg&code_challenge_method=S256&state=ZyXwVuTsRqPoNmLkJiHg"

const interaction: RuntimeInteraction = {
  id: "provider-auth-recovery:login-1", agent_id: "agent-1", kind: "choice", level: "warning",
  title: "Authenticate provider account",
  message: "Open the Claude authorization link to sign in. If Claude gives you a code, paste it below.",
  choices: [{ id: "cancel", label: "Cancel", reply: "cancel", style: "secondary" }],
  custom_choice: { id: "provider-response", label: "Send response", placeholder: "Enter the response requested by the provider CLI", input_kind: "secret", min_length: 1, max_length: 8192 },
  timeout_sec: 600, requested_at_ms: Date.now(),
  provider_login: { kernel_id: "fleet", login: { provider: "claude", account_profile: "disposable-claude-ggpinwmx", login_kind: "terminal_setup_token", login_id: "login-1", auth_url: url }, terminal_output_base64: "" },
}

test("MP-08/MP-11 a login strip shows steps, a countdown, the focused code field and the link alone on its rows", async () => {
  const harness = await createTestRenderer({ width: 100, height: 30, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width: 100 })
  harness.renderer.root.add(box)
  const store = createInteractionChoiceStoreController()
  const clicks: string[] = []
  const logins = createProviderLoginInteractionController({
    getKernelId: async () => "fleet",
    getLoginStatus: () => new Promise(() => {}), getAuthStatus: async () => { throw Error("unused") },
    accountLabel: () => "disposable-claude", localDesktop: () => false, openUrl: async () => false,
    copyUrl: async () => "", showPlainLink: async () => true, pasteCode: () => {},
    appendNotice: () => {}, flashFooter: () => {}, render: () => {}, setTimer: () => null, clearTimer: () => {},
  })
  const render = () => renderAgentInteractionStrips({
    renderer: harness.renderer, primaryBox: box, auxiliaryBoxes: [], visibleAgents: [{ id: "agent-1" }] as never,
    maxAgentsPerScreen: 1, focusedAgentId: "agent-1", activeInteractionForAgent: () => interaction,
    selectedChoiceIndex: store.selectedChoiceIndex, setSelectedChoiceIndex: store.setSelectedIndex,
    customReply: store.customReply, customEditing: store.isCustomEditing,
    queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1, onQueuedPromptAction: () => {},
    providerLoginState: logins.stripState, focusCustomChoiceOnce: store.focusCustomChoiceOnce,
    onProviderLoginLinkClick: (value) => clicks.push(value.id),
  })
  try {
    render()
    await harness.renderOnce()
    const rows = harness.captureCharFrame().split("\n").map((row) => row.trimEnd())
    const first = rows.findIndex((row) => row.includes("1. Open the link (click it):"))
    const second = rows.findIndex((row) => row.includes("2. Authorize Chariox in your browser."))
    assert.ok(first >= 0 && second > first + 1, rows.join("\n"))
    // Only the terminal edge wraps the link; its rows hold nothing else.
    const linkRows = rows.slice(first + 1, second).map((row) => row.slice(1))
    assert.equal(linkRows.join(""), url)
    for (const row of linkRows.slice(0, -1)) assert.equal(row.length, 98)
    assert.match(rows[0]!, /Sign in to Claude · disposable-claude +expires in (10:00|9:5\d)/)
    assert.ok(rows.some((row) => row.includes("3. Paste the code below and press Enter.")))
    assert.ok(rows.some((row) => /> Code: <paste the code>▏ +1\.Cancel +O opens the link · C copies it/.test(row)))
    assert.ok(!rows.some((row) => /Send response|timeout 600s|Login runs on/.test(row)))

    await harness.mockMouse.click(10, first + 2)
    await new Promise((resolve) => setTimeout(resolve, 10))
    assert.deepEqual(clicks, [interaction.id])

    // The kernel asks again (a new request) after a rejected code: focused again.
    store.setSelectedIndex(interaction.id, 0)
    store.setCustomEditing(interaction.id, false)
    render()
    assert.equal(store.isCustomEditing(interaction.id), false, "the same request keeps the user's choice")
    interaction.requested_at_ms += 1
    render()
    assert.equal(store.isCustomEditing(interaction.id), true)
    assert.equal(store.selectedChoiceIndex(interaction.id), 1)
    interaction.requested_at_ms -= 1

    // Typed code is masked; the user's own choice is kept on re-render.
    store.setCustomReply(interaction.id, "code#state")
    store.setSelectedIndex(interaction.id, 0)
    store.setCustomEditing(interaction.id, false)
    render()
    await harness.renderOnce()
    const after = harness.captureCharFrame()
    assert.match(after, / {2}Code: \*{10} +> 1\.Cancel +Enter sends/)
    assert.doesNotMatch(after, /code#state/)
  } finally { harness.renderer.destroy() }
})
