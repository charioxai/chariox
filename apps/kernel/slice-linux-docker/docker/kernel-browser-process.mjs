import {displayGeometry as geometry, pageLimit, chromeBandHeight, featureOn} from './kernel-browser-geometry.mjs';
// MD-2: host Chromium lifetime. Never touches a slice or an existing Chrome.
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { retirePipeBrowserUploads } from './browser-controller-upload-staging.mjs';
import { access, mkdir, readlink, open, mkdtemp, rm } from "node:fs/promises";
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
import { rootBrowserUser, browserUserFailure } from './linux-browser-user.mjs';
import { readBrowserMajor, headlessIdentityArguments } from './kernel-browser-identity.mjs';

function platformPolicy(platform) {
  if (platform === "linux") return linux;
  if (platform === "darwin") return macos;
  throw new Error("MD-2: kernel browser supports Linux and macOS only");
}

// A cold Chromium on a busy host answers its first command after several seconds; later
// commands keep the running browser's request bound.
const STARTUP_ANSWER_MS = 30_000;
export function headlessLaunch(platform, environment) { return platformPolicy(platform).headless(environment); }

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

// RULE: no launch flag and no injected script may change what a page can observe unless a test lists it.
// Pages (Google sign-in, Cloudflare, reCAPTCHA, Turnstile, BotD, BrowserScan) read the browser's identity and its
// automation markers, so every flag below states what, if anything, a page can see of it. The signal probe
// (kernel-browser-signals.mjs, asserted by kernel-browser-standard-chrome.browser-test.mjs against a stock Chrome of the
// same build) covers each visible effect, and kernel-browser-process.test.mjs fails for a flag missing from this table.
// Manual external check before every staging: scripts in kernel-browser-probe.mjs (BrowserScan, Google email step, ...).
export const launchFlagVisibility = {
  '--user-data-dir': 'none',
  // The CDP pipe marks the browser automation-controlled: navigator.webdriver true. Countered by the Blink feature flag below.
  '--remote-debugging-pipe': 'navigator.webdriver (countered)',
  '--no-first-run': 'none', '--no-default-browser-check': 'none', '--hide-crash-restore-bubble': 'none', '--disable-background-networking': 'none',
  '--enable-unsafe-extension-debugging': 'none (the tab-groups component extension is not reachable from a page)',
  '--disable-blink-features': 'navigator.webdriver is false like a Chrome a person starts',
  '--enable-unsafe-swiftshader': 'WebGL exists on a host without a GPU (SwiftShader); a real GPU still wins (listed: webgl)',
  '--enable-features': 'media capabilities: hardware video decode where VA-API exists',
  '--window-size': 'window and screen geometry (listed: screen, window.size)',
  '--headless': 'headless Chrome: HeadlessChrome User-Agent and no pointer; countered by --user-agent and --blink-settings',
  '--user-agent': 'the User-Agent a headed Chrome of the same build and platform sends; drops high-entropy client hints (listed: uad.high)',
  '--blink-settings': 'pointer: fine and hover: hover media queries, as on a desktop with a mouse',
  '--disable-lcd-text': 'text anti-aliasing of the remote panel (canvas text pixels)',
  '--force-device-scale-factor': 'devicePixelRatio, the viewer\'s',
  '--disable-renderer-backgrounding': 'none', '--disable-background-timer-throttling': 'none', '--disable-backgrounding-occluded-windows': 'none',
  '--disable-features': 'none (Chrome\'s own sign-in chip in the band)',
};

// MP-08/MP-10: a private owned window renders at the viewer's device scale
// natively (emulated view scaling renders text differently from CDP).
export function launchArguments(profile, headless, display = false, scale = 1, videoDecode = [], identity = []) {
  return [
    `--user-data-dir=${profile}`, "--remote-debugging-pipe",
    "--no-first-run", "--no-default-browser-check",
    // The host restores its own tabs; after an unclean stop Chromium's
    // "Restore pages?" bubble would cover the page in the owned window.
    "--hide-crash-restore-bubble", "--disable-background-networking",
    // MP-08/MP-11: the kernel loads its tab-groups component extension over this pipe.
    "--enable-unsafe-extension-debugging",
    // The pipe marks the browser as automation-controlled (navigator.webdriver true), which sites such as Google
    // sign-in answer with "This browser or app may not be secure". The person drives this browser like any Chrome.
    "--disable-blink-features=AutomationControlled",
    // Without a usable GPU Chromium refuses WebGL unless software GL is opted in; real GPUs still win.
    "--enable-unsafe-swiftshader", ...videoDecode,
    `--window-size=${geometry.width},${geometry.height+(display?chromeBandHeight:0)}`, ...(headless ? ["--headless=new"] : []),
    // MP-08/MP-10: remote panels have no shared physical LCD subpixel order.
    ...(display ? ["--disable-lcd-text", `--force-device-scale-factor=${scale}`, "--disable-renderer-backgrounding", "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
      // MP-08/MP-11 (BROWSER-UX-14): sign-in is policy-blocked; no dead-end "Sign in to Chrome?" chip in the band.
      "--disable-features=ExplicitBrowserSigninUIOnDesktop,SigninPromoOnAvatarPill"] : []),
    // MP-08/MP-11: no renderer accessibility tree. Masking and the page hit test place the page from CDP and X11.
    ...identity,
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
  constructor(root, { environment = process.env, appStorageRoot } = {}) {
    this.root = root;
    this.appStorageRoot = appStorageRoot;
    this.environment = environment;
    this.child = null;
    this.browserPid = null;
    // A resized browser window must remain inside its owned screen. Its page
    // renders at native density; screenshot DPR does not enlarge this desktop.
    this.desktop = process.platform === "linux" ? new LinuxOwnedDesktop(root, { environment,
      size:{width:pageLimit.width,height:pageLimit.height+chromeBandHeight} }) : null;
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
    this.browserUser ??= this.desktop?.browserUser ?? await rootBrowserUser(this.root);
    if(this.browserUser && !linux.rootPolicyNamespaceAvailable(this.browserUser))throw browserUserFailure();
    const filesRoot = this.browserUser?.root ?? this.root;
    this.filesRoot = filesRoot;
    if (this.browserUser && this.desktop) {
      this.desktop.browserUser = this.browserUser;
      this.desktop.root = filesRoot;
      this.desktop.uid = this.browserUser.uid;
    }
    let environment = platformPolicy(process.platform).launchEnvironment(this.browserUser ? this.browserUser.environment(this.environment) : this.environment);
    const profile = this.browserUser ? await this.browserUser.directory('profile') : path.join(this.root, "profile");
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
    const temporary = this.desktop?.runtime ?? (this.browserUser ? await this.browserUser.directory('temporary') : this.root);
    if (process.platform === 'linux' && !this.browserUser) {
      this.temporaryDirectory = await open(temporary, 'r');
      environment = chromiumTemporaryEnvironment(environment, this.temporaryDirectory.fd);
    }
    const headless = headlessLaunch(process.platform, environment);
    const identity = headless ? headlessIdentityArguments(process.platform, await readBrowserMajor(binary, environment)) : [];
    const args = launchArguments(profile,
      headless, featureOn("CHARIOX_KERNEL_BROWSER_DISPLAY",environment) || featureOn("CHARIOX_KERNEL_BROWSER_MIRROR",environment), this.desktop?.binding() ? 1 : this.scale, platformPolicy(process.platform).videoDecodeArguments(environment), identity);
    this.policy = (this.browserUser || process.platform === "linux" && linux.policyNamespaceAvailable()) ? "managed" : "unavailable";
    if (process.platform === "linux" && this.appStorageRoot && this.policy !== "managed") throw new Error("MP-11: browser isolation unavailable; run Chariox browser Setup to enable private namespaces");
    // Root's mount construction must never write through a browser-writable
    // parent. Chromium only receives the separate owned /tmp bind afterwards.
    if(this.browserUser)this.namespaceRoot=await mkdtemp(path.join(this.root,'browser-namespace-'));
    const launch = this.browserUser
      ? linux.rootPolicyLaunch(await writePolicyLayer(this.root, this.environment), temporary, binary, args, this.browserUser, this.namespaceRoot, undefined, this.appStorageRoot)
      : this.policy === "managed" ? linux.policyLaunch(await writePolicyLayer(this.root, this.environment), temporary, binary, args, undefined, undefined, undefined, this.appStorageRoot) : { command: binary, args };
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
    connection.browserInstanceId = `pipe:${randomUUID()}`;
    if (this.desktop?.binding()) fenceScreenshotPresentation(connection);
    this.connection = connection;
    connection.browserUser = this.browserUser;
    child.once('error', error => { spawnCode=error.code; void connection.close(); });
    child.once('exit', () => { void connection.close(); });
    try {
      if (this.desktop?.binding()) await this.desktop.recordOwned(child, true);
      await connection.send('Browser.getVersion', {}, undefined, { timeoutMs: STARTUP_ANSWER_MS });
      if (this.desktop?.binding()) {
        const owned = await this.desktop.browserProcesses();
        this.browserPid = ownedBrowserPid((await connection.send('SystemInfo.getProcessInfo')).processInfo, owned);
        this.watchBrowserProcess(connection, owned.find(item => item.pid === this.browserPid));
      } else this.browserPid = child.pid;
      child.stderr.off('data',collectStderr);child.stderr.resume();startupStderr='';
      return connection;
    }
    catch(startupError){
      await this.stop(this.connection, { preserveDesktop: keepDesktop });
      const code=browserLaunchFailureReason(startupStderr,spawnCode,startupError?.code);startupStderr='';
      // Once per failed launch, class only: the startup text itself is never logged.
      process.stderr.write(`CHARIOX-LOG browser_launch_failed ${code}\n`);
      throw Object.assign(new Error('MD-2: host Chromium launch failed'),{code});
    }
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
      await this.retireNamespace();
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
    await retirePipeBrowserUploads(connection);
    await this.temporaryDirectory?.close(); this.temporaryDirectory = null;
    await this.retireNamespace();
  }
  async retireNamespace() {
    if(this.namespaceRoot)await rm(this.namespaceRoot,{recursive:true,force:true});
    this.namespaceRoot=null;
  }
}
