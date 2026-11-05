// MP-08/MP-10/MP-11: root keeps control/Vault IPC; graphical programs get only
// their own files and an unprivileged identity. Never open the kernel's home.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { constants } from 'node:fs';
import { mkdir, open, realpath } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
const execute = promisify(execFile);
export const browserAccount = 'chariox-browser';
export const browserUserFailure = () => Object.assign(new Error('MP-08: cannot use the unprivileged browser account; install useradd/getent/setpriv/unshare and allow root to create chariox-browser and private mount namespaces, then retry'), {code:'browser_user_unavailable'});
let account;
export async function ensureBrowserAccount(run = execute) {
  const lookup = async () => {
    const {stdout} = await run('/usr/bin/getent', ['passwd', browserAccount], {timeout:5000});
    const fields = stdout.trim().split(':');
    const uid = Number(fields[2]), gid = Number(fields[3]);
    if (fields.length !== 7 || fields[0] !== browserAccount || ![uid,gid].every(n=>Number.isSafeInteger(n)&&n>0) || !['/usr/sbin/nologin','/sbin/nologin','/bin/false'].includes(fields[6])) throw browserUserFailure();
    return Object.freeze({uid,gid});
  };
  try { return await lookup(); }
  catch {
    // Another kernel can win useradd. Always validate the resulting account.
    await run('/usr/sbin/useradd', ['--system','--user-group','--no-create-home','--home-dir','/nonexistent','--shell','/usr/sbin/nologin',browserAccount], {timeout:5000}).catch(()=>{});
    try { return await lookup(); } catch { throw browserUserFailure(); }
  }
}
// Open without following the leaf; mutate only this directory's descriptor.
// A browser-writable child must never become a root chown/chmod target by path.
export async function ownedBrowserDirectory(directory, uid, gid, mode) {
  await mkdir(directory, {mode}).catch(error=>{if(error.code!=='EEXIST')throw error;});
  const file = await open(directory, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW);
  try {
    const info = await file.stat();
    if (!info.isDirectory() || ![0,uid].includes(info.uid) || await realpath(directory)!==directory) throw browserUserFailure();
    await file.chown(uid,gid); await file.chmod(mode);
  } finally { await file.close(); }
  return directory;
}
export class LinuxBrowserUser {
  constructor(root, identity, base = '/var/lib/chariox-browser') {
    this.uid=identity.uid; this.gid=identity.gid; this.base=base;
    this.root=path.join(base,createHash('sha256').update(root).digest('hex'));
  }
  async prepare() {
    try {
      await ownedBrowserDirectory(this.base,0,0,0o711);
      await ownedBrowserDirectory(this.root,0,0,0o711);
      this.home=await this.directory('home');
      return this;
    } catch { throw browserUserFailure(); }
  }
  directory(name, {uid=this.uid,gid=this.gid,mode=0o700}={}) {
    if(!/^[a-z][a-z-]*$/.test(name))throw browserUserFailure();
    return ownedBrowserDirectory(path.join(this.root,name),uid,gid,mode);
  }
  environment(source) {
    // The root kernel's inherited session, home and provider settings never
    // reach the browser. Display authority is established by the owned desktop.
    const env={};
    for(const key of ['PATH','LANG','LC_ALL','CHARIOX_KERNEL_BROWSER_EXECUTABLE','CHARIOX_KERNEL_BROWSER_HEADLESS','CHARIOX_KERNEL_BROWSER_DISPLAY','CHARIOX_KERNEL_BROWSER_MIRROR','CHARIOX_KERNEL_BROWSER_HOME_URL','LIBVA_DRIVER_NAME'])if(source[key]!==undefined)env[key]=source[key];
    return {...env,HOME:this.home,USER:browserAccount,LOGNAME:browserAccount};
  }
  command(binary,args) {
    return {binary:'/usr/bin/setpriv',args:[`--reuid=${this.uid}`,`--regid=${this.gid}`,'--clear-groups','--inh-caps=-all','--ambient-caps=-all','--',binary,...args]};
  }
}
export async function rootBrowserUser(root) {
  if(process.platform!=='linux'||process.geteuid()!==0)return null;
  account ??= ensureBrowserAccount().catch(error=>{account=null;throw error;});
  return new LinuxBrowserUser(root,await account).prepare();
}
