import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-2: host Chromium lifetime. Never touches a slice or an existing Chrome.
import { spawn } from "node:child_process";
import { access, mkdir, readlink } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import {OwnedDisplay} from "./kernel-browser-owned-display.mjs";
import * as linux from "./kernel-browser-linux.mjs";
import * as macos from "./kernel-browser-macos.mjs";
import { connectCdpPipe } from './kernel-browser-cdp-pipe.mjs';

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

// MP-08/MP-10: the owned window renders at a fixed native density (DSF 2
// at 1280x800): emulated view upscaling would render text differently from
// CDP, while DPR1 pages emulated at view scale 1/2 map 1:1 to its pixels.
export function launchArguments(profile, headless, display = false, scale = 1) {
  return [
    `--user-data-dir=${profile}`, "--remote-debugging-pipe",
    "--no-first-run", "--no-default-browser-check",
    // The host restores its own tabs; after an unclean stop Chromium's
    // "Restore pages?" bubble would cover the page in the owned window.
    "--hide-crash-restore-bubble", "--disable-background-networking",
    `--window-size=${geometry.width},${geometry.height+(display?87:0)}`, ...(headless ? ["--headless=new"] : []),
    // MP-08/MP-10: remote panels have no shared physical LCD subpixel order.
    ...(display ? ["--disable-lcd-text", `--force-device-scale-factor=${scale}`, "--disable-renderer-backgrounding", "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows"] : []), "about:blank",
  ];
}

export class HostChromium {
  constructor(root, { environment = process.env } = {}) {
    this.root = root;
    this.environment = environment;
    this.child = null;
    // MP-08/MP-10/MP-11: the owned window renders at the geometry's native
    // density (DSF 2 at 1280x800). DPR2 viewers read it 1:1; DPR1 viewers
    // are emulated at device scale 1 with view scale 1/2, also 1:1 device
    // pixels. Observation never relaunches the browser for a viewer.
    this.scale = geometry.dpr;
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
      environment.CHARIOX_KERNEL_BROWSER_HEADLESS === "1", environment.CHARIOX_KERNEL_BROWSER_DISPLAY === "1" || environment.CHARIOX_KERNEL_BROWSER_MIRROR === "1", this.scale), {
      stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe'], env: environment,
    });
    this.child = child;
    const connection = connectCdpPipe(child.stdio[3], child.stdio[4]);
    this.connection = connection;
    child.once('error', () => { void connection.close(); });
    child.once('exit', () => { void connection.close(); });
    try { await connection.send('Browser.getVersion'); return connection; }
    catch {}
    await this.stop();
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
