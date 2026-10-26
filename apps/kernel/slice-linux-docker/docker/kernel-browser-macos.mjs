// MD-2: native macOS Chrome, no X server or desktop capture service.
import path from "node:path";
export function candidates(environment) {
  const applications = ["/Applications"];
  if (environment.HOME && path.isAbsolute(environment.HOME)) applications.push(path.join(environment.HOME, "Applications"));
  return applications.flatMap(root => [
    path.join(root, "Chromium.app/Contents/MacOS/Chromium"),
    path.join(root, "Google Chrome.app/Contents/MacOS/Google Chrome"),
    path.join(root, "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
  ]);
}
export function launchEnvironment(environment) {
  const native = { ...environment };
  delete native.DISPLAY;
  delete native.XAUTHORITY;
  delete native.WAYLAND_DISPLAY;
  return native;
}
