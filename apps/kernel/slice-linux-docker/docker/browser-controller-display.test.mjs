import assert from "node:assert/strict";
import test from "node:test";
import { applyBrowserScale, browserScaleChange, managedBrowserHandOut, managedBrowserScale, managedCanonicalDisplay, markBrowserHandedOut } from "./browser-controller-display.mjs";

test("generic local CDP and headless controllers do not require Linux display tooling", () => {
  assert.equal(managedCanonicalDisplay({}), undefined);
  assert.equal(managedCanonicalDisplay({ CHARIOX_SLICE_DISPLAY_MODE: "headless", CHARIOX_SLICE_DISPLAY_SERVER: "Xorg" }), undefined);
  assert.equal(managedCanonicalDisplay({ CHARIOX_SLICE_DISPLAY_MODE: "headed" }), undefined);
});

test("the managed headed Xorg worker enables physical verification", () => {
  assert.equal(typeof managedCanonicalDisplay({ CHARIOX_SLICE_DISPLAY_MODE: "headed", CHARIOX_SLICE_DISPLAY_SERVER: "Xorg" }), "function");
});


test("old managed Xvfb images keep physical verification instead of CDP-only acceptance", () => {
  assert.equal(typeof managedCanonicalDisplay({ CHARIOX_SLICE_DISPLAY_MODE: "headed", CHARIOX_SLICE_ROOT: "/opt/chariox-slice" }), "function");
  assert.equal(typeof managedCanonicalDisplay({ CHARIOX_SLICE_DISPLAY_MODE: "headed", CHARIOX_SLICE_DISPLAY_SERVER: "Xvfb" }), "function");
});

const scaleViewport = { device_scale_factor: 2 };
const answers = (...statuses) => async () => { const status = statuses.shift(); if (status === "down") throw new Error("connection refused"); return { ok: status }; };

test("generic and headless controllers do not relaunch a browser for a scale", () => {
  assert.equal(managedBrowserScale({}), undefined);
  assert.equal(managedBrowserScale({ CHARIOX_SLICE_DISPLAY_MODE: "headless", CHARIOX_SLICE_DISPLAY_SERVER: "Xorg" }), undefined);
  assert.equal(typeof managedBrowserScale({ CHARIOX_SLICE_DISPLAY_MODE: "headed", CHARIOX_SLICE_DISPLAY_SERVER: "Xorg" }), "function");
});

test("a matching browser scale returns at once; a changed one waits for the debugger to answer", async () => {
  const calls = [];
  const run = outcome => async (command, args) => { calls.push([command, ...args.slice(1)]); return { stdout: `${outcome}\n` }; };
  await applyBrowserScale(scaleViewport, {}, "http://127.0.0.1:9222", { run: run("unchanged"), fetchImpl: () => assert.fail("no relaunch, no wait") });
  assert.deepEqual(calls[0], ["/bin/bash", "browser-scale", "2"]);
  let waits = 0;
  await applyBrowserScale(scaleViewport, {}, "http://127.0.0.1:9222", { run: run("relaunching"), fetchImpl: answers("down", false, true), wait: async () => { waits += 1; } });
  assert.equal(waits, 2);
});

test("a browser that does not come back after a scale change is a clear failure", async () => {
  await assert.rejects(
    applyBrowserScale(scaleViewport, {}, "http://127.0.0.1:9222", { run: async () => ({ stdout: "relaunching\n" }), fetchImpl: answers("down", "down"), wait: async () => {}, timeoutMs: -1 }),
    { code: "browser_debugger_unavailable" });
  await assert.rejects(
    applyBrowserScale(scaleViewport, {}, "http://127.0.0.1:9222", { run: async () => { throw new Error("exit 1"); } }),
    { code: "viewport_apply_failed" });
});

test("the scale check only asks whether the running browser would be retired", async () => {
  const calls = [];
  const run = outcome => async (command, args) => { calls.push([command, ...args.slice(1)]); return { stdout: `${outcome}\n` }; };
  for (const outcome of ["relaunch", "unchanged", "handed_out"]) assert.equal(await browserScaleChange(scaleViewport, {}, { run: run(outcome) }), outcome);
  assert.deepEqual(calls[0], ["/bin/bash", "browser-scale", "2", "check"]);
  await assert.rejects(browserScaleChange(scaleViewport, {}, { run: run("relaunching") }), { code: "viewport_apply_failed" });
  await assert.rejects(browserScaleChange(scaleViewport, {}, { run: async () => { throw new Error("exit 2"); } }), { code: "viewport_apply_failed" });
});

test("a handed-out browser lifetime is recorded with the supervisor and never relaunched for a scale", async () => {
  const calls = [];
  await markBrowserHandedOut({}, { run: async (command, args) => { calls.push([command, ...args.slice(1)]); return { stdout: "" }; } });
  assert.deepEqual(calls, [["/bin/bash", "browser-handed-out"]]);
  await assert.rejects(markBrowserHandedOut({}, { run: async () => { throw new Error("exit 1"); } }), { code: "browser_debugger_unavailable" });
  await assert.rejects(
    applyBrowserScale(scaleViewport, {}, "http://127.0.0.1:9222", { run: async () => ({ stdout: "handed_out\n" }), fetchImpl: () => assert.fail("no relaunch") }),
    (error) => error.code === "viewport_apply_failed" && /close the pages/.test(error.message));
  assert.equal(managedBrowserHandOut({}), undefined);
  assert.equal(typeof managedBrowserHandOut({ CHARIOX_SLICE_DISPLAY_MODE: "headed", CHARIOX_SLICE_DISPLAY_SERVER: "Xorg" }), "function");
});
