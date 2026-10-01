import { execFile } from "node:child_process";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { BrowserControllerError } from "./browser-controller-cdp.mjs";

const execute = promisify(execFile);
const helper = fileURLToPath(new URL("./canonical-display.py", import.meta.url));

export function managedCanonicalDisplay(environment = process.env) {
  if (environment.CHARIOX_SLICE_DISPLAY_MODE !== "headed"
      || environment.CHARIOX_SLICE_DISPLAY_SERVER !== "Xorg") return undefined;
  return (viewport) => applyCanonicalDisplay(viewport, {
    ...environment,
    DISPLAY: environment.CHARIOX_SLICE_DISPLAY ?? ":99",
  });
}

export async function applyCanonicalDisplay(viewport, environment = process.env) {
  try {
    const { stdout } = await execute("/opt/chariox-selkies/bin/python", [helper,
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
