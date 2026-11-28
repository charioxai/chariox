// Runs the person driver on the owned fixtures in the three cells (no Chariox web needed): geometry, a typed login, and the drawn canvas.
// Skips with a reason when PW_FROM or the fixtures' Chrome are missing.
import assert from "node:assert/strict"
import test from "node:test"
import { startAll } from "../fixtures/serve.mjs"
import { cells, launch, openClient, regionFacts, typeIn } from "./person-driver.mjs"

test("the person driver types in a fixture form and measures the cross-origin canvas at DPR1, DPR2 and phone", async t => {
  if (!process.env.PW_FROM) return t.skip("set PW_FROM to a node_modules path that holds playwright-core")
  const all = await startAll({ stateDir: null, basePort: 21000 + Math.floor(Math.random() * 3000), altHost: "localhost" })
  const browser = await launch()
  try {
    for (const cell of cells) {
      const client = await openClient(browser, cell)
      await client.page.goto(all.origins.login + "/")
      await typeIn(client.page, client.page.locator("#user"), "alice")
      await typeIn(client.page, client.page.locator("#pass"), "correct horse battery")
      await client.page.locator("#go").click()
      await client.page.waitForURL(/\/welcome$/)
      assert.equal(all.login.submissions.at(-1).pass_len, "correct horse battery".length, cell.id)
      const first = all.login.events.find(e => e.type === "click")
      assert.equal(first.first_click, true, "the first click is flagged and delivered")
      await client.page.goto(all.origins.human + "/")
      const frame = client.page.frameLocator("#grid-frame").locator("#grid")
      await frame.waitFor()
      const facts = await regionFacts(client, frame)
      assert.ok(facts.drawn, `${cell.id}: canvas drawn ${JSON.stringify({ colours: facts.colours, dark: facts.dark_share })}`)
      if (cell.isMobile) assert.ok(facts.whole, "phone: the 300px canvas fits the 390px viewport")
      await client.context.close()
    }
  } finally { await browser.close(); await all.close() }
})
