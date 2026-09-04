import assert from "node:assert/strict"
import test from "node:test"
import vm from "node:vm"
import { roomBrowserClickFixtureScript } from "./room-browser-click-fixture.mjs"

test("only the requested fixture button produces Browser acceptance", () => {
  // Browser DOM is the system boundary. Execute the actual embedded script,
  // with EventTarget dispatch instead of inspecting the generated source.
  const bodyChildren = []
  const mainChildren = []
  const document = new EventTarget()
  document.createElement = () => new EventTarget()
  document.body = { append: (node) => bodyChildren.push(node) }
  document.querySelector = (selector) => {
    assert.equal(selector, "main")
    return { append: (node) => mainChildren.push(node) }
  }
  vm.runInNewContext(roomBrowserClickFixtureScript, { document })
  document.dispatchEvent(new Event("click"))
  assert.equal(mainChildren.some((node) => node.textContent === "BROWSER_CLICK_ACCEPTED"), false)
  const button = bodyChildren.find((node) => node.id === "browser-action-target")
  assert.ok(button)
  button.dispatchEvent(new Event("click"))
  assert.equal(mainChildren.filter((node) => node.textContent === "BROWSER_CLICK_ACCEPTED").length, 1)
})
