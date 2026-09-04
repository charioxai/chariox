import assert from "node:assert/strict"
import http from "node:http"
import test from "node:test"
import { roomProviderBrowserFixture } from "./room-provider-browser-fixture.mjs"
import { captureBrowserSnapshot } from "../../../kernel/slice-linux-docker/docker/browser-controller-snapshot.mjs"
import { performBrowserAction } from "../../../kernel/slice-linux-docker/docker/browser-controller-actions.mjs"

// Explicit local browser check. No downloads or provider usage. The caller
// supplies the already-installed Playwright module and Chrome.
assert.ok(process.env.PLAYWRIGHT_MODULE, "set PLAYWRIGHT_MODULE to the installed Playwright module; this test never downloads dependencies")
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE)

test("nested-frame form controls exist only inside two frames and submit to the top page", async () => {
  let browser
  const fixtureOptions = { mode: "browser", browserTask: "form", browserLayout: "nested-frame" }
  const server = http.createServer((request, response) => {
    const fixture = roomProviderBrowserFixture(fixtureOptions, request.url)
    response.writeHead(200, { "content-type": "text/html" })
    response.end(`<main><div id="state">READY</div></main><script>${fixture.script}</script>`)
  })
  try {
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve))
    browser = await chromium.launch({ channel: "chrome", headless: true })
    const page = await browser.newPage()
    await page.goto(`http://127.0.0.1:${server.address().port}/click`)
    assert.equal(await page.getByLabel("Browser sample").count(), 0, "no top-document alternative may satisfy the task")
    const inner = page.frameLocator("iframe").frameLocator("iframe")
    await inner.getByLabel("Browser sample").waitFor()
    await exerciseControllerForm(page)
    await page.waitForURL(/browser_sample=Chariox\+form\+sample/)
    assert.equal(await page.getByText("BROWSER_FORM_ACCEPTED", { exact: true }).count(), 1)
  } finally {
    await browser?.close()
    await new Promise((resolve) => server.close(resolve))
  }
})

test("shadow-root form controls have no light-DOM alternative and submit successfully", async () => {
  let browser
  const fixtureOptions = { mode: "browser", browserTask: "form", browserLayout: "shadow-root" }
  const server = http.createServer((request, response) => {
    const fixture = roomProviderBrowserFixture(fixtureOptions, request.url)
    response.writeHead(200, { "content-type": "text/html" })
    response.end(`<main><div id="state">READY</div></main><script>${fixture.script}</script>`)
  })
  try {
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve))
    browser = await chromium.launch({ channel: "chrome", headless: true })
    const page = await browser.newPage()
    await page.goto(`http://127.0.0.1:${server.address().port}/click`)
    assert.equal(await page.locator('xpath=//input[@name="browser_sample"]').count(), 0)
    // Controller fill into shadow DOM has a separate focus regression.
    // Keep this case checking discovery and the fixture until that fix lands.
    await exerciseControllerForm(page, false)
    await page.getByLabel("Browser sample").fill("Chariox form sample")
    await page.getByRole("button", { name: "Submit Browser form", exact: true }).click()
    await page.waitForURL(/browser_sample=Chariox\+form\+sample/)
    assert.equal(await page.getByText("BROWSER_FORM_ACCEPTED", { exact: true }).count(), 1)
  } finally {
    await browser?.close()
    await new Promise((resolve) => server.close(resolve))
  }
})

async function exerciseControllerForm(page, submit = true) {
  const cdp = await page.context().newCDPSession(page)
  try {
    const { frameTree } = await cdp.send("Page.getFrameTree")
    const context = {
      connection: { send: (method, params) => cdp.send(method, params) },
      sessionId: "fixture",
      targetId: frameTree.frame.id,
      documentId: frameTree.frame.loaderId,
      browserGeneration: 1,
      snapshotRevision: 1,
    }
    const snapshot = await captureBrowserSnapshot(context)
    const controls = []
    for (const [role, name] of [["textbox", "Browser sample"], ["button", "Submit Browser form"]]) {
      const matches = snapshot.accessibility_nodes.filter((node) => !node.ignored && node.role === role && node.name.trim() === name)
      assert.equal(matches.length, 1, `controller must discover exactly one ${role} named ${name}`)
      assert.ok(snapshot.dom_nodes.some((node) => node.node_ref === matches[0].node_ref), "accessible control must resolve to a DOM node")
      controls.push(matches[0].node_ref)
    }
    if (submit) {
      await performBrowserAction({ ...context, nodeRef: controls[0], action: { kind: "fill", text: "Chariox form sample" } })
      await performBrowserAction({ ...context, nodeRef: controls[1], action: { kind: "submit" } })
    }
  } finally {
    await cdp.detach()
  }
}
