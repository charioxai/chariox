// MP-08 / MP-11: one lazy owned desktop per kernel user, never the login session.
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { randomBytes, randomUUID } from 'node:crypto';
import path from 'node:path';
import os from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { processIdentity, settleOwned, descendants } from './linux-owned-process.mjs';

export function desktopEnvironment(source, runtime, uid = process.getuid?.()) {
  if (!Number.isInteger(uid) || uid <= 0) throw new Error('MP-11: owned desktop requires a non-root kernel user');
  const env = {};
  for (const key of ['PATH', 'HOME', 'USER', 'LOGNAME', 'LANG', 'LC_ALL', 'TMPDIR', 'CHARIOX_KERNEL_BROWSER_EXECUTABLE', 'CHARIOX_KERNEL_BROWSER_HEADLESS', 'CHARIOX_KERNEL_BROWSER_DISPLAY', 'CHARIOX_KERNEL_BROWSER_MIRROR', 'CHARIOX_BROWSER_DISPLAY_PYTHON', 'CHARIOX_BROWSER_DISPLAY_TIMING']) if (source[key] !== undefined) env[key] = source[key];
  return { ...env, XAUTHORITY: path.join(runtime, 'Xauthority'), XDG_RUNTIME_DIR: runtime, NO_AT_BRIDGE: '0', GTK_A11Y: 'always' };
}
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
  constructor(root, { environment = process.env, uid = process.getuid?.(), commands = {} } = {}) {
    this.root = root; this.source = environment; this.uid = uid; this.commands = commands;
    this.children = []; this.current = null; this.starting = null;
  }
  binding() { return this.current; }
  async launch(name, args, env, stdio = ['ignore', 'ignore', 'ignore']) {
    const child = spawn(this.commands[name] ?? name, args, { env, stdio });
    // Attach before the first asynchronous boundary to prevent unhandled spawn errors.
    child.on('error', () => {});
    const record = { child, identity: null, core: ['Xvfb', 'openbox', 'dbus-daemon'].includes(name) }; this.children.push(record);
    if (!child.pid) throw new Error('MP-08: desktop executable unavailable');
    record.identity = await processIdentity(child.pid);
    if (!record.identity) throw new Error('MP-11: desktop child ownership unavailable');
    return child;
  }
  async ownedProcesses() {
    const roots=this.children.filter(({child,identity})=>identity && child.exitCode===null && child.signalCode===null).map(({identity})=>identity);
    return descendants(roots);
  }
  async recordOwned(child) {
    const identity=await processIdentity(child.pid);
    if(!identity)throw new Error('MP-11: graphical process ownership unavailable');
    this.children.push({child,identity,core:false});
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
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    this.runtime = await mkdtemp(path.join(this.root, 'desktop-'));
    try {
      const env = desktopEnvironment(this.source, this.runtime, this.uid);
      const cookie = randomBytes(16);
      await writeFile(env.XAUTHORITY, authorityCookie(cookie), { mode: 0o600, flag: 'wx' });
      const x = await this.launch('Xvfb', ['-displayfd', '3', '-screen', '0', '1280x800x24', '-auth', env.XAUTHORITY, '-nolisten', 'tcp', '-nolisten', 'local', '-noreset'], env, ['ignore', 'ignore', 'ignore', 'pipe']);
      const display = await firstLine(x.stdio[3], x);
      if (!/^\d{1,5}$/.test(display)) throw new Error('MP-11: invalid owned display');
      await writeFile(env.XAUTHORITY, authorityCookie(cookie, display), { mode: 0o600 });
      cookie.fill(0);
      env.DISPLAY = `:${display}`;
      const busPath = path.join(this.runtime, 'bus');
      const bus = await this.launch('dbus-daemon', ['--session', '--nofork', `--address=unix:path=${busPath}`, '--print-address=3'], env, ['ignore', 'ignore', 'ignore', 'pipe']);
      const address = await firstLine(bus.stdio[3], bus);
      if (!address.startsWith(`unix:path=${busPath},guid=`)) throw new Error('MP-11: invalid owned session bus');
      env.DBUS_SESSION_BUS_ADDRESS = address;
      const keymap=await this.launch('setxkbmap',['-layout','us'],env);
      const keymapExit=keymap.exitCode??await new Promise(resolve=>keymap.once('exit',resolve));
      if(keymapExit!==0)throw new Error('MP-08: owned display keymap unavailable');
      await this.launch('openbox', ['--sm-disable'], env);
      await delay(100);
      if (this.children.some(({child,core}) => core && (child.exitCode !== null || child.signalCode !== null))) throw new Error('MP-08: desktop failed to become ready');
      this.current = Object.freeze({ surface_id: `desktop-${randomUUID()}`, generation: randomUUID(), ownedProcesses: () => this.ownedProcesses(), width: 1280, height: 800, environment: Object.freeze(env) });
      return this.current;
    } catch (error) { await this.stop(); throw error; }
  }
  async stop() {
    this.current = null;
    await settleOwned(this.children.filter(({child}) => child.pid));
    this.children = [];
    if (this.runtime) await rm(this.runtime, { recursive: true, force: true });
    this.runtime = null;
  }
}
