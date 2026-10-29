// MP-08 / MP-11: one lazy owned desktop per kernel user, never the login session.
import { spawn } from 'node:child_process';
import { access, mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { randomBytes, randomUUID } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import os from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { desktopApplication } from './desktop-applications.mjs';
import { namespaceLauncher } from './kernel-browser-linux.mjs';
import { processIdentity, settleOwned, descendants, observedDescendants } from './linux-owned-process.mjs';
const readyDesktops = new WeakSet();
export const ownsDesktopDisplay = desktop => readyDesktops.has(desktop) && Boolean(desktop.binding()) &&
  Number.isSafeInteger(desktop.child?.pid) && desktop.child.pid > 1 && desktop.child.exitCode === null && desktop.child.signalCode === null;

export function desktopEnvironment(source, runtime, uid = process.getuid?.()) {
  if (!Number.isInteger(uid) || uid <= 0) throw new Error('MP-11: owned desktop requires a non-root kernel user');
  const env = {};
  for (const key of ['PATH', 'HOME', 'USER', 'LOGNAME', 'LANG', 'LC_ALL', 'TMPDIR', 'CHARIOX_KERNEL_BROWSER_EXECUTABLE', 'CHARIOX_KERNEL_BROWSER_HEADLESS', 'CHARIOX_KERNEL_BROWSER_DISPLAY', 'CHARIOX_KERNEL_BROWSER_MIRROR', 'CHARIOX_BROWSER_DISPLAY_PYTHON', 'CHARIOX_BROWSER_DISPLAY_TIMING', 'CHARIOX_BROWSER_DISPLAY_OPENH264', 'CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER', 'CHARIOX_BROWSER_DISPLAY_SOFTWARE', 'LIBVA_DRIVER_NAME', 'CHARIOX_BROWSER_DISPLAY_STRIPE_WORKERS']) if (source[key] !== undefined) env[key] = source[key];
  // MP-08 / MP-11: Chromium's native ATK bridge has a separate enablement
  // check from renderer accessibility. Keep both on this owned desktop.
  return { ...env, CHARIOX_OWNED_VIRTUAL_DISPLAY: '1', TMPDIR: runtime, XAUTHORITY: path.join(runtime, 'Xauthority'), XDG_RUNTIME_DIR: runtime, NO_AT_BRIDGE: '0', GTK_A11Y: 'always', ACCESSIBILITY_ENABLED: '1' };
}
export const desktopBusAddress = () => `unix:abstract=chariox-desktop-${randomUUID()}`;
export const desktopCommand = (binary, args) => ({ binary: '/usr/bin/python3',
  // Ubuntu's userns exception belongs to the installed launcher. Bubblewrap's
  // private Xvfb /tmp must enter through it too, before creating its namespace.
  args: [fileURLToPath(new URL('./linux-desktop-session.py', import.meta.url)),
    ...(binary === 'bwrap' ? [namespaceLauncher(), '--user', '--map-root-user', '--mount', '--'] : []),
    binary, ...args] });
function authorityCookie(cookie, display) {
  // FamilyWild; the cookie is valid only on this server, whose displayfd is not yet known.
  const field = value => { const size = Buffer.alloc(2); size.writeUInt16BE(value.length); return Buffer.concat([size, value]); };
  return Buffer.concat([display === undefined ? Buffer.from([255, 255]) : Buffer.from([1, 0]), field(display === undefined ? Buffer.alloc(0) : Buffer.from(os.hostname())), field(display === undefined ? Buffer.alloc(0) : Buffer.from(display)), field(Buffer.from('MIT-MAGIC-COOKIE-1')), field(cookie)]);
}
async function firstLine(stream, child) {
  return new Promise((resolve, reject) => {
    let data = '';
    const timer = setTimeout(() => finish(new Error('MP-08: desktop readiness timeout')), 5000);
    const finish = (error, line) => { clearTimeout(timer); stream.off('data', onData); child.off('exit', onExit); child.off('error', onError); error ? reject(error) : resolve(line); };
    const onData = chunk => { data += chunk; if (data.length > 4096) finish(new Error('MP-11: invalid desktop readiness')); else if (data.includes('\n')) finish(null, data.split('\n')[0]); };
    const onExit = () => finish(new Error('MP-08: desktop child exited before readiness'));
    const onError = () => finish(new Error('MP-08: desktop executable unavailable'));
    stream.on('data', onData); child.once('exit', onExit); child.once('error', onError);
  });
}
export class LinuxOwnedDesktop {
  constructor(root, { environment = process.env, commands = {} } = {}) {
    this.root = root; this.source = environment; this.uid = process.getuid?.(); this.commands = commands;
    this.children = []; this.known = new Map(); this.browserKnown = new Map(); this.current = null; this.starting = null;
  }
  binding() { return this.current; }
  isLive() { return readyDesktops.has(this) && Boolean(this.current) && !this.children.some(({child,core})=>core&&(child.exitCode!==null||child.signalCode!==null)); }
  get child() { return this.children.find(item => item.name === 'bwrap')?.child ?? null; }
  get environment() { return this.current?.environment; }
  async launch(name, args, env, stdio = ['ignore', 'ignore', 'ignore']) {
    const core = ['Xvfb', 'bwrap', 'openbox', 'dbus-daemon'].includes(name);
    const command = core ? desktopCommand(this.commands[name] ?? name, args)
      : { binary: this.commands[name] ?? name, args };
    const child = spawn(command.binary, command.args, { env, stdio });
    // Attach before the first asynchronous boundary to prevent unhandled spawn errors.
    child.on('error', () => {});
    const record = { child, identity: null, core, name }; this.children.push(record);
    if (!child.pid) throw new Error('MP-08: desktop executable unavailable');
    record.identity = await processIdentity(child.pid);
    if (!record.identity) throw new Error('MP-11: desktop child ownership unavailable');
    return child;
  }
  // MP-08/MP-11: installed descriptors are resolved inside this desktop user.
  async installedApplications({signal}={}) {
    const binding=this.current;
    if(!binding)throw Error('MP-08: native desktop unavailable');
    const result=await desktopApplication({op:'list'},binding.environment,signal);
    if(this.current!==binding)throw Error('MP-11: stale application desktop');
    return result;
  }
  async launchApplication(applicationId,{signal,observe}={}) {
    const binding=this.current;
    if(!binding)throw Error('MP-08: native desktop unavailable');
    const app=await desktopApplication({op:'resolve',application_id:applicationId},binding.environment,signal);
    if(signal?.aborted || this.current!==binding)throw Error('MP-11: application launch cancelled');
    const command=desktopCommand(app.argv[0],app.argv.slice(1));
    const child=await this.launch(command.binary,command.args,binding.environment);
    const record=this.children.find(item=>item.child===child);
    const stop=()=>void settleOwned([record]);
    signal?.addEventListener('abort',stop,{once:true});
    try {
      if(signal?.aborted || this.current!==binding)throw Error('MP-11: application launch cancelled');
      const observation=await observe?.();
      if(signal?.aborted || this.current!==binding)throw Error('MP-11: application launch cancelled');
      return {application_id:app.application_id,name:app.name,launched:true,...(observation?{observation}:{})};
    } catch(error) {await settleOwned([record]);throw error;}
    finally {signal?.removeEventListener('abort',stop);}

  }
  async renderProcesses() {
    const roots=this.children.filter(({child,identity})=>identity&&child.exitCode===null&&child.signalCode===null);
    const processes=await observedDescendants(roots.map(({identity})=>identity));
    const browserIds=new Set(roots.filter(({browser})=>browser).map(({identity})=>identity.pid));
    for(let changed=true;changed;){changed=false;for(const item of processes)if(!browserIds.has(item.pid)&&browserIds.has(item.parent)){browserIds.add(item.pid);changed=true}}
    for(const item of processes)if(browserIds.has(item.pid))this.browserKnown.set(item.pid,item);
    return {processes,browser_processes:processes.filter(item=>browserIds.has(item.pid))};
  }
  async ownedProcesses() {
    const roots=this.children.filter(({child,identity})=>identity && child.exitCode===null && child.signalCode===null).map(({identity})=>identity);
    const current=await descendants(roots);
    for(const item of current)this.known.set(item.pid,item);
    return current;
  }
  async recordOwned(child, browser = false) {
    const identity=await processIdentity(child.pid);
    if(!identity)throw new Error('MP-11: graphical process ownership unavailable');
    this.children.push({child,identity,core:false,browser});
  }
  async browserProcesses() {
    const current=await descendants(this.children.filter(({child,identity,browser})=>browser && identity && child.exitCode===null && child.signalCode===null).map(({identity})=>identity));
    for(const item of current)this.browserKnown.set(item.pid,item);
    return current;
  }
  // MP-08/MP-10: a browser crash does not retire the user's other apps or X11
  // identity. Use only verified browser roots/descendants, including orphans
  // already observed before their supervisor exited.
  async stopBrowser() {
    const browser=this.children.filter(item=>item.browser);
    const known=await descendants([...browser.map(item=>item.identity).filter(Boolean),...this.browserKnown.values()]);
    for(const item of known)this.browserKnown.set(item.pid,item);
    await settleOwned(browser,[...this.browserKnown.values()]);
    this.children=this.children.filter(item=>!browser.includes(item));
    for(const item of this.browserKnown.values())if(this.known.get(item.pid)?.started===item.started)this.known.delete(item.pid);
    this.browserKnown.clear();
  }
  async start() {
    if (this.current) {
      if (this.children.some(({ child, core }) => core && (child.exitCode !== null || child.signalCode !== null))) throw new Error('MP-08: owned desktop unavailable; explicitly stop/start');
      return this.current;
    }
    if (this.starting) return this.starting;
    this.starting = this.create();
    try { return await this.starting; } finally { this.starting = null; }
  }
  async create() {
    desktopEnvironment(this.source, this.root, this.uid);
    // MP-08/MP-10/MP-11: the Python supervisor otherwise hides a missing
    // executable behind a child-readiness exit. Name only fixed prerequisites,
    // never the operator's command overrides or private PATH entries.
    for (const name of ['bwrap', 'Xvfb', 'dbus-daemon', 'openbox', 'setxkbmap']) {
      const binary = this.commands[name] ?? name;
      const candidates = binary.includes('/') ? [binary] : (this.source.PATH ?? '/bin:/usr/bin').split(path.delimiter).map(root => path.join(root || '.', binary));
      let available = false;
      for (const candidate of candidates) {
        try { await access(candidate, 1); available = true; break; } catch {}
      }
      if (!available) throw new Error(`MP-08: owned desktop prerequisite unavailable (${name})`);
    }
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    this.runtime = await mkdtemp(path.join(this.root, 'desktop-'));
    try {
      const env = desktopEnvironment(this.source, this.runtime, this.uid);
      const cookie = randomBytes(16);
      await writeFile(env.XAUTHORITY, authorityCookie(cookie), { mode: 0o600, flag: 'wx' });
      // MP-08 / MP-11: Xvfb hardcodes /tmp for sockets and compiled keymaps,
      // ignoring TMPDIR. Bind only its owned runtime there; no global temp
      // permissions are required. Keep the network/IPC namespaces shared so
      // ordinary clients use the cookie-authenticated abstract X11 socket.
      // Its exclusive bind also keeps displayfd allocation safe across kernels.
      const x = await this.launch('bwrap', [
        '--dev-bind', '/', '/', '--bind', this.runtime, '/tmp', '--die-with-parent', '--',
        this.commands.Xvfb ?? 'Xvfb', '-displayfd', '3', '-screen', '0', '1280x800x24',
        '-auth', '/tmp/Xauthority', '-nolisten', 'tcp', '-noreset',
      ], { ...env, TMPDIR: '/tmp', XAUTHORITY: '/tmp/Xauthority' }, ['ignore', 'ignore', 'ignore', 'pipe']);
      const display = await firstLine(x.stdio[3], x);
      x.stdio[3].destroy();
      if (!/^\d{1,5}$/.test(display)) throw new Error('MP-11: invalid owned display');
      await writeFile(env.XAUTHORITY, authorityCookie(cookie, display), { mode: 0o600 });
      cookie.fill(0);
      env.DISPLAY = `:${display}`;
      // Filesystem socket paths fail for long kernel state roots; AT-SPI's
      // fallback listener also hardcodes /tmp. Own both EXTERNAL-authenticated
      // buses explicitly, with short unique abstract addresses.
      const startBus = async (options, addressKey) => {
        const address = desktopBusAddress();
        // Activated registry children inherit the bus daemon's launch env.
        // They must discover this owned bus rather than starting a default one.
        env[addressKey] = address;
        const bus = await this.launch('dbus-daemon', [...options, '--nofork', `--address=${address}`, '--print-address=3'], env, ['ignore', 'ignore', 'ignore', 'pipe']);
        const ready = await firstLine(bus.stdio[3], bus);
        bus.stdio[3].destroy();
        if (!ready.startsWith(`${address},guid=`)) throw new Error('MP-11: invalid owned desktop bus');
        return ready;
      };
      env.DBUS_SESSION_BUS_ADDRESS = await startBus(['--session'], 'DBUS_SESSION_BUS_ADDRESS');
      const a11yConfigs = ['/etc/at-spi2/accessibility.conf', '/usr/share/defaults/at-spi2/accessibility.conf'];
      const config = await (async () => { for (const file of a11yConfigs) { try { await access(file); return file; } catch (error) { if (error.code !== 'ENOENT') throw error; } } })();
      if (!config) throw new Error('MP-08: AT-SPI bus configuration unavailable');
      env.AT_SPI_BUS_ADDRESS = await startBus([`--config-file=${config}`], 'AT_SPI_BUS_ADDRESS');
      const keymap=await this.launch('setxkbmap',['-layout','us'],env);
      const keymapExit=keymap.exitCode??await new Promise(resolve=>keymap.once('exit',resolve));
      if(keymapExit!==0)throw new Error('MP-08: owned display keymap unavailable');
      await this.launch('openbox', ['--sm-disable'], env);
      await delay(100);
      const failed=this.children.find(({child,core}) => core && (child.exitCode !== null || child.signalCode !== null));
      if(failed)throw new Error(`MP-08: desktop failed to become ready (${failed.name}, exit ${failed.child.exitCode ?? 'signal'})`);
      this.current = Object.freeze({ surface_id: `desktop-${randomUUID()}`, generation: randomUUID(), ownedProcesses: () => this.ownedProcesses(), browserProcesses: () => this.browserProcesses(), renderProcesses: () => this.renderProcesses(), browser: () => this.browser?.() ?? null, width: 1280, height: 800, environment: Object.freeze(env) });
      readyDesktops.add(this);
      return this.current;
    } catch (error) { await this.stop(); throw error; }
  }
  async stop() {
    readyDesktops.delete(this);
    this.current = null;
    await settleOwned(this.children.filter(({child}) => child.pid), [...this.known.values()]);
    this.children = [];this.known.clear();this.browserKnown.clear();
    if (this.runtime) await rm(this.runtime, { recursive: true, force: true });
    this.runtime = null;
  }
  close() { return this.stop(); }
}
