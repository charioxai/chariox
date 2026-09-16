import assert from "node:assert/strict"
import test from "node:test"
import { roomProviderBrowserFixture } from "./room-provider-browser-fixture.mjs"

test("Browser form acceptance comes from the submitted value, not a click", () => {
  const options = { mode: "browser", browserTask: "form" }
  const initial = roomProviderBrowserFixture(options, "/click")
  assert.equal(initial.initialClicks, 0)
  assert.match(initial.script, /Browser sample/)
  assert.match(initial.script, /Submit Browser form/)
  assert.doesNotMatch(initial.script, /BROWSER_FORM_ACCEPTED/)
  const accepted = roomProviderBrowserFixture(options, "/click?browser_sample=Chariox+form+sample")
  assert.equal(accepted.initialClicks, 1)
  assert.match(accepted.script, /BROWSER_FORM_ACCEPTED/)
  const rejected = roomProviderBrowserFixture(options, "/click?browser_sample=wrong")
  assert.equal(rejected.initialClicks, 0)
  assert.doesNotMatch(rejected.script, /BROWSER_FORM_ACCEPTED/)
  assert.equal(roomProviderBrowserFixture({ mode: "computer" }, "/click?browser_sample=Chariox+form+sample").script, "")
})
