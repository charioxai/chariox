import path from "node:path";

const LOCAL_X_DISPLAY = /^:\d+(?:\.\d+)?$/;

export function browserControllerLaunchOptions(env = {}) {
  const requestedMode = env.CHARIOX_TEST_CHROMIUM_MODE;
  const mode = requestedMode === undefined ? "headless" : requestedMode;
  if (mode !== "headless" && mode !== "headed") {
    throw new Error("CHARIOX_TEST_CHROMIUM_MODE must be headless or headed");
  }

  const configuredPath = env.CHARIOX_TEST_CHROMIUM;
  if (configuredPath !== undefined && typeof configuredPath !== "string") {
    throw new Error("CHARIOX_TEST_CHROMIUM must be an absolute executable path");
  }
  const executablePath = configuredPath?.trim() ?? "";
  if (executablePath && !path.isAbsolute(executablePath)) {
    throw new Error("CHARIOX_TEST_CHROMIUM must be an absolute executable path");
  }

  if (
    mode === "headed"
    && (typeof env.DISPLAY !== "string" || !LOCAL_X_DISPLAY.test(env.DISPLAY))
  ) {
    throw new Error("headed mode requires DISPLAY to be a local X display such as :99");
  }

  return {
    headless: mode === "headless",
    args: ["--remote-debugging-port=0", "--site-per-process"],
    ...(executablePath ? { executablePath } : { channel: "chrome" }),
  };
}
