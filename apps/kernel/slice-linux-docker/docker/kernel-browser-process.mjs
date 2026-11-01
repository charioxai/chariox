import {displayGeometry as geometry, chromeBandHeight, featureOn} from './kernel-browser-geometry.mjs';
// MD-2: host Chromium lifetime. Never touches a slice or an existing Chrome.
import { spawn } from "node:child_process";
import { access, mkdir, readlink, open } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import {OwnedDisplay} from "./kernel-browser-owned-display.mjs";
import * as linux from "./kernel-browser-linux.mjs";
import * as macos from "./kernel-browser-macos.mjs";
import { LinuxOwnedDesktop, desktopCommand } from './linux-owned-desktop.mjs';
import { connectCdpPipe } from './kernel-browser-cdp-pipe.mjs';
import { fenceScreenshotPresentation } from './browser-capture-presentation.mjs';
import { writePolicyLayer } from './kernel-browser-policy.mjs';
import { browserLaunchFailureReason } from './kernel-browser-error-label.mjs';
import { isOwnedAlive, validOwnedPid } from './linux-owned-process.mjs';
import { requireBrowserMemory } from './browser-memory.mjs';

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

// MP-08/MP-10: a private owned window renders at the viewer's device scale
// natively (emulated view scaling renders text differently from CDP).
export function launchArguments(profile, headless, display = false, scale = 1) {
  return [
    `--user-data-dir=${profile}`, "--remote-debugging-pipe",
    "--no-first-run", "--no-default-browser-check",
    // The host restores its own tabs; after an unclean stop Chromium's
    // "Restore pages?" bubble would cover the page in the owned window.
    "--hide-crash-restore-bubble", "--disable-background-networking",
    // MP-08/MP-11: the kernel loads its tab-groups component extension over this pipe.
    "--enable-unsafe-extension-debugging",
    // Without a usable GPU Chromium refuses WebGL unless software GL is opted in; real GPUs still win.
    "--enable-unsafe-swiftshader",
    `--window-size=${geometry.width},${geometry.height+(display?chromeBandHeight:0)}`, ...(headless ? ["--headless=new"] : []),
    // MP-08/MP-10: remote panels have no shared physical LCD subpixel order.
    ...(display ? ["--disable-lcd-text", `--force-device-scale-factor=${scale}`, "--disable-renderer-backgrounding", "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
      // MP-08/MP-11 (BROWSER-UX-14): sign-in is policy-blocked; no dead-end "Sign in to Chrome?" chip in the band.
      "--disable-features=ExplicitBrowserSigninUIOnDesktop,SigninPromoOnAvatarPill"] : []),
    // MP-08/MP-11: no renderer accessibility tree. Masking and the page hit test place the page from CDP and X11.
    "about:blank",
  ];
}

// MP-08 / MP-11: Chromium's singleton socket must fit sockaddr_un even when
// the owned state path is long. This is the same private directory, held open
// by the adapter until Chromium and its children have settled.
export function chromiumTemporaryEnvironment(environment, fd, pid = process.pid) {
  if (!Number.isSafeInteger(fd) || fd < 0 || !Number.isSafeInteger(pid) || pid <= 1) throw new Error('MP-11: invalid private temporary directory');
  return { ...environment, TMPDIR: `/proc/${pid}/fd/${fd}` };
}

// MP-11: CDP identifies Chromium, while the launch PID may be its supervisor.
// Accept exactly one browser process already in the desktop's verified tree.
export function ownedBrowserPid(processInfo, owned) {
  const browsers = Array.isArray(processInfo) ? processInfo.filter(item => item.type === 'browser') : [];
  const pid = browsers[0]?.id;
  if (browsers.length !== 1 || !Number.isSafeInteger(pid) || pid <= 1 || !owned.some(item => item.pid === pid))
    throw Error('MP-11: owned browser PID unavailable');
  return pid;
}

export class HostChromium {
  constructor(root, { environment = process.env } = {}) {
    this.root = root;
    this.environment = environment;
    this.child = null;
    this.browserPid = null;
    this.desktop = process.platform === "linux" ? new LinuxOwnedDesktop(root, { environment }) : null;
    // MP-08/MP-10: on the shared owned desktop Chromium is one window at the
    // desktop's scale; only a private owned display may rescale it.
    this.scale = this.desktop ? 1 : geometry.dpr;
  }
  stopWatchingBrowser() {
    clearTimeout(this.browserWatch?.timer);
    this.browserWatch = null;
  }
  // MP-08/MP-10: a supervisor or orphan can hold CDP open after Chrome dies.
  // Observe only the verified browser identity; this watcher never signals it.
  watchBrowserProcess(connection, identity, { alive = isOwnedAlive, interval = 500 } = {}) {
    if (!validOwnedPid(identity?.pid) || !identity.started) throw new Error('MP-11: missing owned browser identity');
    this.stopWatchingBrowser();
    const watch = this.browserWatch = { connection, identity };
    const check = async () => {
      if (this.browserWatch !== watch || this.connection !== connection) return;
      if (connection.isOpen?.() === false) { this.stopWatchingBrowser(); return; }
      let live = true;
      try { live = await alive(identity); } catch {} // a temporary proc read failure is retried
      if (this.browserWatch !== watch || this.connection !== connection) return;
      if (!live) { this.stopWatchingBrowser(); await connection.close(); return; }
      watch.timer = setTimeout(check, interval); watch.timer.unref?.();
    };
    watch.timer = setTimeout(check, interval); watch.timer.unref?.();
  }
  async start({ preserveDesktop = false } = {}) {
    const keepDesktop = preserveDesktop && this.desktop?.isLive?.() === true;
    if (this.child && this.child.exitCode === null && this.child.signalCode === null) {
      if (this.connection?.isOpen()) return this.connection;
      await this.stop(this.connection, { preserveDesktop: keepDesktop });
    }
    // MP-08/MP-11: settle a previously owned display before failed recovery.
    requireBrowserMemory();
    if (!keepDesktop) { await this.display?.close(); this.display = null; }
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
    const headedRenderer = featureOn('CHARIOX_KERNEL_BROWSER_DISPLAY',environment) || featureOn('CHARIOX_KERNEL_BROWSER_MIRROR',environment);
    if (this.desktop && environment.CHARIOX_KERNEL_BROWSER_HEADLESS !== "1") {
      environment = (await this.desktop.start()).environment;
      this.display = this.desktop;
    } else
    // MP-08/MP-10/MP-11: the private headed browser belongs to the kernel,
    // independently of whether a video encoder/transport is enabled.
    if(process.platform==='linux'&&headedRenderer&&environment.CHARIOX_KERNEL_BROWSER_HEADLESS!=='1'){
      // MP-11: the kernel reclaims this transient root even after supervisor SIGKILL.
      this.display=new OwnedDisplay(environment.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT || this.root);
      environment={...environment};
      for(const name of ['DISPLAY','XAUTHORITY','WAYLAND_DISPLAY'])delete environment[name];
      try{environment={...environment,...await this.display.start()};}
      catch(error){await this.display.close();this.display=null;throw error;}
    }
    const temporary = this.desktop?.runtime ?? this.root;
    if (process.platform === 'linux') {
      this.temporaryDirectory = await open(temporary, 'r');
      environment = chromiumTemporaryEnvironment(environment, this.temporaryDirectory.fd);
    }
    const args = launchArguments(profile,
      environment.CHARIOX_KERNEL_BROWSER_HEADLESS === "1", featureOn("CHARIOX_KERNEL_BROWSER_DISPLAY",environment) || featureOn("CHARIOX_KERNEL_BROWSER_MIRROR",environment), this.desktop?.binding() ? 1 : this.scale);
    this.policy = process.platform === "linux" && linux.policyNamespaceAvailable() ? "managed" : "unavailable";
    const launch = this.policy === "managed" ? linux.policyLaunch(await writePolicyLayer(this.root, this.environment), temporary, binary, args) : { command: binary, args };
    const command = this.desktop?.binding() ? desktopCommand(launch.command, launch.args) : { binary: launch.command, args: launch.args };
    const child = spawn(command.binary, command.args, {
      stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'], env: environment,
    });
    this.child = child;
    child.on('error', () => {});
    // MP-11: startup text remains bounded in memory and is never logged/replied.
    let startupStderr='', spawnCode;
    const collectStderr=chunk=>{if(startupStderr.length<8192)startupStderr+=chunk.toString('utf8').slice(0,8192-startupStderr.length);};
    child.stderr.on('data',collectStderr);
    const connection = connectCdpPipe(child.stdio[3], child.stdio[4]);
    if (this.desktop?.binding()) fenceScreenshotPresentation(connection);
    this.connection = connection;
    child.once('error', error => { spawnCode=error.code; void connection.close(); });
    child.once('exit', () => { void connection.close(); });
    try {
      if (this.desktop?.binding()) await this.desktop.recordOwned(child, true);
      await connection.send('Browser.getVersion');
      if (this.desktop?.binding()) {
        const owned = await this.desktop.browserProcesses();
        this.browserPid = ownedBrowserPid((await connection.send('SystemInfo.getProcessInfo')).processInfo, owned);
        this.watchBrowserProcess(connection, owned.find(item => item.pid === this.browserPid));
      } else this.browserPid = child.pid;
      child.stderr.off('data',collectStderr);child.stderr.resume();startupStderr='';
      return connection;
    }
    catch {}
    await this.stop(this.connection, { preserveDesktop: keepDesktop });
    const code=browserLaunchFailureReason(startupStderr,spawnCode);startupStderr='';
    throw Object.assign(new Error('MD-2: host Chromium launch failed'),{code});
  }
  async stop(connection = this.connection, { preserveDesktop = false } = {}) {
    this.stopWatchingBrowser();
    const keepDesktop = preserveDesktop && this.desktop?.isLive?.() === true;
    const child = this.child;
    this.child = null;
    this.browserPid = null;
    this.connection = null;
    if (!Number.isSafeInteger(child?.pid) || child.pid <= 1) {
      if (!keepDesktop) { await this.display?.close();this.display=null; }
      if (child?.pid !== undefined) throw new Error("MD-2: refusing unsafe browser process ID");
      if (keepDesktop) await this.desktop.stopBrowser(); else await this.desktop?.stop();
      await this.temporaryDirectory?.close(); this.temporaryDirectory = null;
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
    if (keepDesktop) await this.desktop.stopBrowser(); else { await this.desktop?.stop(); this.display = null; }
    await this.temporaryDirectory?.close(); this.temporaryDirectory = null;
  }
}
