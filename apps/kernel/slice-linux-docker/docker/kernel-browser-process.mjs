import {displayGeometry as geometry,hostDisplayScale} from './kernel-browser-geometry.mjs';
// MD-2: host Chromium lifetime. Never touches a slice or an existing Chrome.
import { spawn } from "node:child_process";
import { access, mkdir, readFile, readlink } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import {OwnedDisplay} from "./kernel-browser-owned-display.mjs";
import * as linux from "./kernel-browser-linux.mjs";
import * as macos from "./kernel-browser-macos.mjs";
import { connectCdpPipe } from './kernel-browser-cdp-pipe.mjs';

// Only these fixed diagnostics can cross the host error-sanitization boundary.
// Browser stderr may contain page/profile data and is never returned or logged.
export class HostChromiumSandboxError extends Error {
  constructor(restricted = false) {
    super(restricted
      ? "MD-2: Chromium has no usable sandbox while AppArmor restricts unprivileged user namespaces (kernel.apparmor_restrict_unprivileged_userns=1). Set CHARIOX_KERNEL_BROWSER_EXECUTABLE to system Chrome/Chromium installed with its AppArmor profile (for example /opt/google/chrome/chrome), or ask an administrator to install a profile permitting userns for this executable or a compatible SUID sandbox. Keep the browser sandbox and host restriction enabled."
      : "MD-2: Chromium has no usable sandbox. Use system Chrome/Chromium with its sandbox support, or ask an administrator to install an AppArmor userns profile or compatible SUID sandbox. Keep the browser sandbox enabled.");
  }
}

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

export function launchArguments(profile, headless, display = false) {
  return [
    `--user-data-dir=${profile}`, "--remote-debugging-pipe",
    "--no-first-run", "--no-default-browser-check",
    "--disable-session-crashed-bubble", "--disable-background-networking",
    `--window-size=${display?geometry.width*geometry.dpr/hostDisplayScale:geometry.width},${display?geometry.height*geometry.dpr/hostDisplayScale+87:geometry.height}`, ...(headless ? ["--headless=new"] : []),
    // MP-08/MP-10: remote panels have no shared physical LCD subpixel order.
    ...(display ? ["--disable-lcd-text", `--force-device-scale-factor=${hostDisplayScale}`, "--disable-renderer-backgrounding", "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows"] : []), "about:blank",
  ];
}

export class HostChromium {
  constructor(root, { environment = process.env } = {}) {
    this.root = root;
    this.environment = environment;
    this.child = null;
  }
  async start() {
    if (this.child && this.child.exitCode === null && this.child.signalCode === null) {
      if (this.connection?.isOpen()) return this.connection;
      await this.stop();
    }
    await this.display?.close();this.display=null;
    let environment = platformPolicy(process.platform).launchEnvironment(this.environment);
    const profile = path.join(this.root, "profile");
    await mkdir(profile, { recursive: true, mode: 0o700 });
    // Refuse a live profile owner before launching Chromium. Never
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
    const binary=await executable(environment);
    if(process.platform==='linux'&&environment.CHARIOX_KERNEL_BROWSER_DISPLAY==='1'&&environment.CHARIOX_KERNEL_BROWSER_HEADLESS!=='1'){
      // MP-11: the kernel reclaims this transient root even after supervisor SIGKILL.
      this.display=new OwnedDisplay(environment.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT || this.root);
      try{environment={...environment,...await this.display.start()};}catch{this.display=null;}
    }
    const child = spawn(binary, launchArguments(profile,
      environment.CHARIOX_KERNEL_BROWSER_HEADLESS === "1", environment.CHARIOX_KERNEL_BROWSER_DISPLAY === "1" || environment.CHARIOX_KERNEL_BROWSER_MIRROR === "1"), {
      stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'], env: environment,
    });
    let stderrTail = '', sandboxFailed = false;
    child.stderr.on('data', chunk => {
      const text = stderrTail + chunk.toString('utf8');
      sandboxFailed ||= /No usable sandbox|SUID sandbox helper binary.*not configured correctly/i.test(text);
      stderrTail = text.slice(-128);
    });
    this.child = child;
    const connection = connectCdpPipe(child.stdio[3], child.stdio[4]);
    this.connection = connection;
    child.once('error', () => { void connection.close(); });
    child.once('exit', () => { void connection.close(); });
    try { await connection.send('Browser.getVersion'); await this.display?.parkPointer(); return connection; }
    catch {}
    await this.stop();
    if (sandboxFailed) {
      let restricted = false;
      if (process.platform === 'linux') {
        try { restricted = (await readFile('/proc/sys/kernel/apparmor_restrict_unprivileged_userns', 'utf8')).trim() === '1'; } catch {}
      }
      throw new HostChromiumSandboxError(restricted);
    }
    throw new Error("MD-2: sandboxed host Chromium did not become ready (check executable, display and profile ownership)");
  }
  async stop(connection = this.connection) {
    const child = this.child;
    this.child = null;
    this.connection = null;
    if (!Number.isSafeInteger(child?.pid) || child.pid <= 1) {
      await this.display?.close();this.display=null;
      if (child?.pid !== undefined) throw new Error("MD-2: refusing unsafe browser process ID");
      return;
    }
    if (connection?.isOpen()) await connection.send("Browser.close").catch(() => {});
    const exited = () => child.exitCode !== null || child.signalCode !== null;
    for (let count = 0; count < 40 && !exited(); count++) await delay(50);
    if (!exited()) {
      if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("MP-11: invalid owned browser PID");
      child.kill("SIGTERM");
    }
    for (let count = 0; count < 40 && !exited(); count++) await delay(50);
    if (!exited()) {
      if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("MP-11: invalid owned browser PID");
      child.kill("SIGKILL");
    }
    if (!exited()) await new Promise(resolve => child.once("exit", resolve));
    await connection?.close();
    await this.display?.close();this.display=null;
  }
}
