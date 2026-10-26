// A synthetic Chromium listener with real Linux lifetime and upload staging.
// The lifetime survives controller SIGKILL; the owning Rust fixture retires it.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export async function fixtureUploadBrowser(directory, root) {
  const { BrowserUploadStaging } = await import(pathToFileURL(join(directory, "browser-controller-upload-staging.mjs")));
  process.env.CHARIOX_BROWSER_LIFECYCLE_ROOT = join(root, "upload-browser-lifetimes");
  // Keep the lifecycle helper default upload reaper inside this owned fixture.
  process.env.TMPDIR = root;
  const profile = join(root, "upload-browser-profile");
  const receipt = join(root, "upload-browser-lifetime.json");
  const portFile = join(root, "upload-browser-port");
  let pending;
  const ensure = () => pending ??= (async () => {
    if (process.platform !== "linux") throw new Error("secure upload fixture requires Linux browser ownership");
    let record;
    if (existsSync(receipt)) {
      record = JSON.parse(readFileSync(receipt, "utf8"));
      // MP-08/MP-10/MP-11: reject unsafe PIDs even for liveness probes.
      if (!Number.isSafeInteger(record?.browser?.pid) || record.browser.pid <= 1) throw new Error("unsafe owned browser PID");
      process.kill(record.browser.pid, 0);
    } else {
      const code = `import socket,time\ns=socket.socket();s.bind(("127.0.0.1",0));s.listen()\nopen(${JSON.stringify(portFile)},"w").write(str(s.getsockname()[1]))\ntime.sleep(180)`;
      record = JSON.parse(execFileSync("python3", [join(directory, "browser-lifecycle.py"), "start", profile,
        join(root, "upload-browser.log"), "python3", "-c", code, `--user-data-dir=${profile}`],
        { encoding: "utf8", timeout: 8000 }));
      writeFileSync(receipt, JSON.stringify(record), { mode: 0o600 });
    }
    let port;
    for (let tries = 0; tries < 100 && !port; tries++) {
      if (existsSync(portFile)) port = Number(readFileSync(portFile, "utf8"));
      if (!port) await new Promise(resolve => setTimeout(resolve, 10));
    }
    if (!Number.isInteger(port) || port <= 0) throw new Error("owned upload browser listener is unavailable");
    return {
      browserInstanceId: `ws://127.0.0.1:${port}/devtools/browser/browser-${record.instance}`,
      processInfo: { processInfo: [{ type: "browser", id: record.browser.pid }] },
    };
  })();
  return { ensure, stageUploads: options => new BrowserUploadStaging({
    root: join(root, `chariox-browser-uploads-${process.getuid()}`),
  }).prepare(options) };
}
