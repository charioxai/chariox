import { execFile } from "node:child_process";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { setTimeout as delay } from "node:timers/promises";
import { BrowserControllerError } from "./browser-controller-cdp.mjs";

const execute = promisify(execFile);
const helper = fileURLToPath(new URL("./canonical-display.py", import.meta.url));
const sliceScreen = fileURLToPath(new URL("./slice-screen.sh", import.meta.url));
// Chromium takes a while to open its debugging port after a relaunch.
const BROWSER_RELAUNCH_TIMEOUT_MS = 45_000;

function managedHeadedSlice(environment) {
  const managed = environment.CHARIOX_SLICE_DISPLAY_SERVER === "Xorg"
    || environment.CHARIOX_SLICE_DISPLAY_SERVER === "Xvfb"
    || environment.CHARIOX_SLICE_ROOT === "/opt/chariox-slice";
  return environment.CHARIOX_SLICE_DISPLAY_MODE === "headed" && managed;
}

export function managedCanonicalDisplay(environment = process.env) {
  if (!managedHeadedSlice(environment)) return undefined;
  return (viewport) => applyCanonicalDisplay(viewport, {
    ...environment,
    DISPLAY: environment.CHARIOX_SLICE_DISPLAY ?? ":99",
    CHARIOX_SLICE_DISPLAY_SERVER: environment.CHARIOX_SLICE_DISPLAY_SERVER ?? "Xvfb",
  });
}

// The slice browser is launched at the viewer's scale (a window at another
// native scale shows the emulated page in one corner of the screen). A scale
// change relaunches it; this resolves once its debugging port answers again.
export function managedBrowserScale(environment = process.env, debuggerEndpoint = "http://127.0.0.1:9222") {
  if (!managedHeadedSlice(environment)) return undefined;
  return (viewport) => applyBrowserScale(viewport, { ...environment, DISPLAY: environment.CHARIOX_SLICE_DISPLAY ?? ":99" }, debuggerEndpoint);
}

export async function applyBrowserScale(viewport, environment, debuggerEndpoint, { run = execute, fetchImpl = fetch, wait = delay, timeoutMs = BROWSER_RELAUNCH_TIMEOUT_MS } = {}) {
  let outcome;
  try {
    ({ stdout: outcome } = await run("/bin/bash", [sliceScreen, "browser-scale", String(viewport.device_scale_factor)],
      { timeout: 35_000, maxBuffer: 1024, env: environment }));
  } catch {
    throw new BrowserControllerError("viewport_apply_failed", "browser scale apply failed; the Room browser could not be relaunched at the viewer scale");
  }
  if (outcome.trim() !== "relaunching") return;
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      if ((await fetchImpl(new URL("/json/version", debuggerEndpoint), { signal: AbortSignal.timeout(2_000) })).ok) return;
    } catch {}
    if (Date.now() >= deadline) {
      throw new BrowserControllerError("browser_debugger_unavailable", "the Room browser did not come back after its scale change; retry the viewport update");
    }
    await wait(250);
  }
}

export async function applyCanonicalDisplay(viewport, environment = process.env) {
  try {
    const { stdout } = await execute(environment.CHARIOX_SLICE_VIEWER_BACKEND === "novnc" ? "/usr/bin/python3" : "/opt/chariox-selkies/bin/python", [helper,
      String(viewport.desktop_pixel_width), String(viewport.desktop_pixel_height)],
    { timeout: 35_000, maxBuffer: 1024, env: environment });
    const actual = JSON.parse(stdout);
    if (actual.width !== viewport.desktop_pixel_width || actual.height !== viewport.desktop_pixel_height) {
      throw new Error("display mismatch");
    }
  } catch {
    throw new BrowserControllerError("viewport_apply_failed", "canonical physical display apply failed");
  }
}
