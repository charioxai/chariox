import assert from "node:assert/strict"
import test from "node:test"
import { roomCompanionClickFixture } from "./room-companion-click-fixture.mjs"
import { roomProviderBrowserFixture } from "./room-provider-browser-fixture.mjs"

test("human-only companion starts at a fresh click page and expects one click", () => {
  assert.deepEqual(roomCompanionClickFixture(null), {
    path: "/click",
    readyMarker: "POINTER_CLICK_READY",
    pointerClickExpectedCount: 1,
  })
})

for (const realProvider of [
  {},
  { mode: "computer" },
  { mode: "browser" },
  { mode: "browser", browserTask: "click" },
]) {
  test(`provider ${JSON.stringify(realProvider)} click plus human takeover requires two clicks`, () => {
    const setup = roomCompanionClickFixture(realProvider)
    assert.equal(roomProviderBrowserFixture(realProvider, setup.path).initialClicks, 0)
    assert.equal(setup.pointerClickExpectedCount, 2)
  })
}

for (const browserLayout of ["page", "nested-frame", "shadow-root"]) {
  for (const browserMutation of [undefined, "replace-field"]) {
    test(`companion ${browserLayout} form ${browserMutation ?? "submit"} preserves the provider result before human takeover`, () => {
      const realProvider = { mode: "browser", browserTask: "form", browserLayout, browserMutation }
      const setup = roomCompanionClickFixture(realProvider)
      assert.equal(setup.path, "/click", "preparation must clear prior form acceptance")
      assert.equal(setup.readyMarker, "POINTER_CLICK_READY")
      const initial = roomProviderBrowserFixture(realProvider, setup.path)
      assert.equal(initial.initialClicks, 0)
      assert.doesNotMatch(initial.script, /BROWSER_FORM_ACCEPTED/)

      const accepted = roomProviderBrowserFixture(realProvider,
        "/click?browser_sample=Chariox+form+sample&browser_replaced=1&browser_stale_safe=1")
      assert.equal(accepted.initialClicks, 1, "provider form navigation starts its accepted page at one")
      assert.match(accepted.script, /BROWSER_FORM_ACCEPTED/)
      assert.equal(setup.pointerClickExpectedCount, 2, "the later human click increments the accepted page")
    })
  }
}

test("office companion retains its separate physical-effect contract", () => {
  assert.equal(roomCompanionClickFixture({ mode: "computer", computerTask: "office" }).pointerClickExpectedCount, 1)
})
