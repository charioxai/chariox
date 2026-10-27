import assert from "node:assert/strict"
import test from "node:test"
import vm from "node:vm"
import { roomCompanionClickFixture, roomClickFixtureCounterScript } from "./room-companion-click-fixture.mjs"
import { roomProviderBrowserFixture } from "./room-provider-browser-fixture.mjs"

test("human-only companion starts at a fresh click page and expects one click", () => {
  assert.deepEqual(roomCompanionClickFixture(null), {
    path: "/click?room-companion-reset=1",
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
      assert.equal(setup.path, "/click?room-companion-reset=1", "preparation must clear prior form acceptance")
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

function loadCounter({ requestUrl, storage, initialClicks = 0 }) {
  const state = { textContent: "POINTER_CLICK_READY" }
  const listeners = new Map()
  const location = { href: new URL(requestUrl, "http://fixture.invalid").href }
  vm.runInNewContext(roomClickFixtureCounterScript({
    storageKey: "this-run", initialClicks, requestUrl,
  }), {
    URL,
    location,
    history: {
      state: { retained: true },
      replaceState: (_state, _title, url) => { location.href = String(url) },
    },
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
      removeItem: (key) => storage.delete(key),
    },
    document: {
      querySelector: () => state,
      addEventListener: (event, listener) => listeners.set(event, listener),
      body: { style: {} },
    },
  })
  return {
    state,
    location,
    click: (gesture = false) => listeners.get("click")({
      target: { closest: () => gesture ? {} : null },
    }),
  }
}

test("explicit companion preparation resets the prior physical click without clearing unrelated storage", () => {
  const storage = new Map([["this-run", "1"], ["other-run", "7"], ["browser-auth", "preserved"]])
  const setup = roomCompanionClickFixture(null)
  const page = loadCounter({ requestUrl: setup.path, storage })
  assert.equal(page.state.textContent, setup.readyMarker)
  assert.equal(storage.has("this-run"), false)
  assert.equal(storage.get("other-run"), "7")
  assert.equal(storage.get("browser-auth"), "preserved")
  page.click(true)
  assert.equal(storage.has("this-run"), false, "gesture controls must not count as clicks")
  page.click()
  assert.equal(page.state.textContent, "POINTER_CLICK_COUNT=1")
  assert.equal(storage.get("this-run"), "1")
})

for (const requestUrl of ["/click", "/click?browser_sample=Chariox+form+sample", "/click?room-companion-reset=0"]) {
  test(`normal navigation ${requestUrl} preserves physical clicks for reload and persistence checks`, () => {
    const storage = new Map([["this-run", "2"], ["other-run", "7"]])
    const page = loadCounter({ requestUrl, storage, initialClicks: 1 })
    assert.equal(page.state.textContent, "POINTER_CLICK_COUNT=2")
    assert.equal(storage.get("this-run"), "2")
    page.click()
    assert.equal(page.state.textContent, "POINTER_CLICK_COUNT=3")
    assert.equal(storage.get("other-run"), "7")
  })
}

test("companion reset is consumed once so a subsequent reload preserves the new click", () => {
  const storage = new Map([["this-run", "1"]])
  const setup = roomCompanionClickFixture(null)
  const page = loadCounter({ requestUrl: setup.path + "&retained=value", storage })
  assert.equal(new URL(page.location.href).searchParams.has("room-companion-reset"), false)
  assert.equal(new URL(page.location.href).searchParams.get("retained"), "value")
  page.click()
  const reloaded = loadCounter({ requestUrl: page.location.href, storage })
  assert.equal(reloaded.state.textContent, "POINTER_CLICK_COUNT=1")
  reloaded.click()
  assert.equal(reloaded.state.textContent, "POINTER_CLICK_COUNT=2")
})
