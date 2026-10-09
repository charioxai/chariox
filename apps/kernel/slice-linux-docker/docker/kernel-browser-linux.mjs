// MD-2: Linux host policy. The renderer sandbox is never disabled.
export function candidates() {
  return ["/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome"];
}
export function launchEnvironment(environment, uid = process.getuid?.()) {
  if (uid === 0) throw new Error("MD-2: run the kernel browser as a normal Unix user; root Chromium is unsupported");
  return environment;
}
