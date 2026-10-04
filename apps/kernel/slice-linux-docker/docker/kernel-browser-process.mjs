// MD-2: host Chromium lifetime. Never touches a slice or an existing Chrome.
import { spawn } from "node:child_process";
import { access, mkdir, readFile, readlink, unlink } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import * as linux from "./kernel-browser-linux.mjs";
import * as macos from "./kernel-browser-macos.mjs";

function platformPolicy(platform) {
  if (platform === "linux") return linux;
  if (platform === "darwin") return macos;
  throw new Error("MD-2: kernel browser supports Linux and macOS only");
}

export async function executable(environment = process.env, platform = process.platform) {
  const candidates = environment.CHARIOX_KERNEL_BROWSER_EXECUTABLE
    ? [environment.CHARIOX_KERNEL_BROWSER_EXECUTABLE]
    : platformPolicy(platform).candidates(environment);
  for (const candidate of candidates) {
    if (!path.isAbsolute(candidate)) throw new Error("MD-2: browser executable must be absolute");
    try { await access(candidate, 1); return candidate; } catch {}
  }
  throw new Error("MD-2: install native Chromium or set CHARIOX_KERNEL_BROWSER_EXECUTABLE");
}

export function launchArguments(profile, headless) {
  return [
    `--user-data-dir=${profile}`, "--remote-debugging-address=127.0.0.1",
    "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
    "--disable-session-crashed-bubble", "--disable-background-networking",
    "--window-size=1280,800", ...(headless ? ["--headless=new"] : []), "about:blank",
  ];
}

export class HostChromium {
  constructor(root, { environment = process.env } = {}) {
    this.root = root;
    this.environment = environment;
    this.child = null;
  }
  async start() {
    if (this.child && this.child.exitCode === null && this.child.signalCode === null) return this.endpoint;
    const environment = platformPolicy(process.platform).launchEnvironment(this.environment);
    const profile = path.join(this.root, "profile");
    await mkdir(profile, { recursive: true, mode: 0o700 });
    // Refuse a live profile owner before touching its debugger marker. Never
    // adopt an existing Chromium, delete singleton locks, or steal its profile.
    let lock;
    try { lock = await readlink(path.join(profile, "SingletonLock")); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
    if (lock) {
      const pid = Number(lock.split("-").at(-1));
      if (!Number.isInteger(pid) || pid <= 1) throw new Error("MD-2: invalid browser profile lock");
      let alive = true;
      try { process.kill(pid, 0); } catch (error) {
        if (error.code === "ESRCH") alive = false;
        else if (error.code !== "EPERM") throw error;
      }
      if (alive) throw new Error("MD-2: browser profile is already owned by a live process");
    }
    // This marker belongs only to this profile. Chromium's singleton lock stays intact.
    await unlink(path.join(profile, "DevToolsActivePort")).catch(error => {
      if (error.code !== "ENOENT") throw error;
    });
    const child = spawn(await executable(environment), launchArguments(profile,
      environment.CHARIOX_KERNEL_BROWSER_HEADLESS === "1"), {
      stdio: "ignore", env: environment,
    });
    this.child = child;
    let spawnFailed = false;
    child.once("error", () => { spawnFailed = true; });
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      if (spawnFailed || child.exitCode !== null || child.signalCode !== null) break;
      try {
        const [port] = (await readFile(path.join(profile, "DevToolsActivePort"), "utf8")).split("\n");
        if (/^[0-9]+$/.test(port) && +port > 0 && +port <= 65535) {
          const endpoint = `http://127.0.0.1:${port}`;
          const response = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(1000) });
          if (response.ok) { this.endpoint = endpoint; return endpoint; }
        }
      } catch {}
      await delay(50);
    }
    await this.stop();
    throw new Error("MD-2: sandboxed host Chromium did not become ready (check executable, display and profile ownership)");
  }
  async stop(connection) {
    const child = this.child;
    this.child = null;
    if (!child?.pid) return;
    if (connection?.isOpen()) await connection.send("Browser.close").catch(() => {});
    const exited = () => child.exitCode !== null || child.signalCode !== null;
    for (let count = 0; count < 40 && !exited(); count++) await delay(50);
    if (!exited()) child.kill("SIGTERM");
    for (let count = 0; count < 40 && !exited(); count++) await delay(50);
    if (!exited()) child.kill("SIGKILL");
    if (!exited()) await new Promise(resolve => child.once("exit", resolve));
  }
}
