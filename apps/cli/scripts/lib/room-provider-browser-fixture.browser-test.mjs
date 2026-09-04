import assert from "node:assert/strict"
import http from "node:http"
import test from "node:test"
import { roomProviderBrowserFixture } from "./room-provider-browser-fixture.mjs"

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
    await inner.getByLabel("Browser sample").fill("Chariox form sample")
    await inner.getByRole("button", { name: "Submit Browser form", exact: true }).click()
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
    await page.getByLabel("Browser sample").fill("Chariox form sample")
    await page.getByRole("button", { name: "Submit Browser form", exact: true }).click()
    await page.waitForURL(/browser_sample=Chariox\+form\+sample/)
    assert.equal(await page.getByText("BROWSER_FORM_ACCEPTED", { exact: true }).count(), 1)
  } finally {
    await browser?.close()
    await new Promise((resolve) => server.close(resolve))
  }
})
