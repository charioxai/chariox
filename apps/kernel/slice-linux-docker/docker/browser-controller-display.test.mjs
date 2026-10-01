import assert from "node:assert/strict";
import test from "node:test";
import { managedCanonicalDisplay } from "./browser-controller-display.mjs";

test("generic local CDP and headless controllers do not require Linux display tooling", () => {
  assert.equal(managedCanonicalDisplay({}), undefined);
  assert.equal(managedCanonicalDisplay({ CHARIOX_SLICE_DISPLAY_MODE: "headless", CHARIOX_SLICE_DISPLAY_SERVER: "Xorg" }), undefined);
  assert.equal(managedCanonicalDisplay({ CHARIOX_SLICE_DISPLAY_MODE: "headed" }), undefined);
});

test("only the explicitly managed headed Xorg worker enables physical verification", () => {
  assert.equal(typeof managedCanonicalDisplay({ CHARIOX_SLICE_DISPLAY_MODE: "headed", CHARIOX_SLICE_DISPLAY_SERVER: "Xorg" }), "function");
});
