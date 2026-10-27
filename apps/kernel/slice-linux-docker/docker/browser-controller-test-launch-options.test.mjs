import assert from "node:assert/strict";
import test from "node:test";
import { browserControllerLaunchOptions } from "./browser-controller-test-launch-options.mjs";

test("missing launch settings keep the headless Playwright Chrome default", () => {
  assert.deepEqual(browserControllerLaunchOptions({}), {
    channel: "chrome",
    headless: true,
    args: ["--remote-debugging-port=0", "--site-per-process"],
  });
  assert.deepEqual(browserControllerLaunchOptions({ CHARIOX_TEST_CHROMIUM_MODE: "headless" }), {
    channel: "chrome",
    headless: true,
    args: ["--remote-debugging-port=0", "--site-per-process"],
  });
});

test("headed mode selects the explicit browser and keeps the controller flags", () => {
  assert.deepEqual(browserControllerLaunchOptions({
    CHARIOX_TEST_CHROMIUM: " /usr/bin/chromium ",
    CHARIOX_TEST_CHROMIUM_MODE: "headed",
    DISPLAY: ":99",
  }), {
    executablePath: "/usr/bin/chromium",
    headless: false,
    args: ["--remote-debugging-port=0", "--site-per-process"],
  });
});

test("only the documented browser mode values are accepted", () => {
  for (const mode of ["", "true", "1", "HEADLESS", null]) {
    assert.throws(
      () => browserControllerLaunchOptions({ CHARIOX_TEST_CHROMIUM_MODE: mode }),
      /CHARIOX_TEST_CHROMIUM_MODE must be headless or headed/,
    );
  }
});

test("headed mode rejects missing and malformed local X displays", () => {
  for (const display of [undefined, "", "  ", "99", ":x", "remote:99"]) {
    assert.throws(
      () => browserControllerLaunchOptions({
        CHARIOX_TEST_CHROMIUM_MODE: "headed",
        DISPLAY: display,
      }),
      /headed mode requires DISPLAY/,
    );
  }
});

test("Chromium override stays optional but must be absolute when provided", () => {
  assert.deepEqual(browserControllerLaunchOptions({ CHARIOX_TEST_CHROMIUM: "  \t" }), {
    channel: "chrome",
    headless: true,
    args: ["--remote-debugging-port=0", "--site-per-process"],
  });
  assert.throws(
    () => browserControllerLaunchOptions({ CHARIOX_TEST_CHROMIUM: "relative/chromium" }),
    /absolute executable path/,
  );
});
