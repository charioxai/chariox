// MP-08/MP-10/MP-11: root keeps control/Vault IPC; graphical programs get only
// their own files and an unprivileged identity. Never open the kernel's home.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { constants, readFileSync } from 'node:fs';
import { lstat, mkdir, open, readdir, readlink, realpath, rename } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { browserUserMessages } from './kernel-browser-error-label.mjs';
const execute = promisify(execFile);
export const browserAccount = 'chariox-browser';
export const browserUserFailure = (reason='account') => Object.assign(new Error(browserUserMessages[reason]), {code:'browser_user_unavailable'});

function readProcessAttribute(path) {
  try { return readFileSync(path,'utf8'); } catch { return ''; }
}

export function browserConfinementFailure(capability) {
  const profile = readProcessAttribute('/proc/self/attr/current').trim().split(/\s+/)[0];
  if (process.env.CHARIOX_KERNEL_STATE_CONFINED === '1' || profile.split('//&').some(part=>part==='bwrap'||part==='unpriv_bwrap')) return browserUserFailure('nested');
  const effective = /^CapEff:\s*([0-9a-f]+)$/im.exec(readProcessAttribute('/proc/self/status'))?.[1];
  const bit = capability === 'namespaceCapability' ? 21n : 0n;
  if (capability && effective && (BigInt(`0x${effective}`) & (1n << bit)) === 0n) return browserUserFailure(capability);
  return browserUserFailure('host');
}

let account, reclaimed;
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
// Row 61: each kernel's browser dir carries a record of the kernel root it
// belongs to, so a later kernel start can reclaim it once that root is gone.
const ownerRecord='owner.json';
const browserDirectoryName=/^[0-9a-f]{64}$/;
const reclaimPrefix='.reclaim-';
const rootDigest=root=>createHash('sha256').update(root).digest('hex');
// Written by root inside the root-owned 0711 dir: the browser account can't create or replace it.
async function writeOwnerRecord(directory, root) {
  const file=path.join(directory,ownerRecord), temporary=`${file}.${process.pid}.tmp`;
  const handle=await open(temporary,constants.O_WRONLY|constants.O_CREAT|constants.O_TRUNC|constants.O_NOFOLLOW,0o600);
  try { await handle.chmod(0o600); await handle.writeFile(JSON.stringify({version:1,root})); } finally { await handle.close(); }
  await rename(temporary,file);
}
// A record counts only if it is a regular file of the kernel's account, not a
// link, and names a root that hashes to this very dir.
async function recordedRoot(directory, owner) {
  let handle;
  try { handle=await open(path.join(directory,ownerRecord),constants.O_RDONLY|constants.O_NOFOLLOW|constants.O_NONBLOCK); } catch { return null; }
  try {
    const info=await handle.stat();
    if(!info.isFile()||info.uid!==owner||(info.mode&0o022)||info.size>4096)return null;
    const record=JSON.parse(await handle.readFile('utf8'));
    return record?.version===1&&typeof record.root==='string'&&path.isAbsolute(record.root)&&rootDigest(record.root)===path.basename(directory)?record.root:null;
  } catch { return null; } finally { await handle.close(); }
}
async function profileInUse(directory) {
  const lock=await readlink(path.join(directory,'profile','SingletonLock')).catch(()=>'');
  const pid=Number(lock.slice(lock.lastIndexOf('-')+1));
  if(!lock||!Number.isSafeInteger(pid)||pid<=1)return false;
  try { process.kill(pid,0); return true; } catch (error) { return error.code==='EPERM'; }
}
// The browser account owns the profile trees (and every kernel's Chromium runs
// as it), so removal must never walk them by path: GNU rm descends with openat
// and never follows a symlink; --one-file-system keeps it off other mounts.
const removeBrowserTree=directory=>execute('/bin/rm',['-rf','--one-file-system','--',directory],{timeout:120000});
export async function sweepBrowserHomes(base, {owner=0, removeTree=removeBrowserTree}={}) {
  const removed=[];
  const baseInfo=await lstat(base).catch(()=>null);
  if(!baseInfo?.isDirectory()||baseInfo.uid!==owner)return {removed};
  for(const name of await readdir(base).catch(()=>[])){
    const entry=path.join(base,name);
    const info=await lstat(entry).catch(()=>null);
    if(!info?.isDirectory()||info.uid!==owner)continue;
    // An earlier start renamed it for removal and stopped before finishing.
    if(name.startsWith(reclaimPrefix)&&browserDirectoryName.test(name.slice(reclaimPrefix.length))){await removeTree(entry).catch(()=>{});continue;}
    if(!browserDirectoryName.test(name))continue;
    const root=await recordedRoot(entry,owner);
    if(!root||!await lstat(root).then(()=>false,error=>error.code==='ENOENT')||await profileInUse(entry))continue;
    const doomed=path.join(base,reclaimPrefix+name);
    try { await rename(entry,doomed); } catch { continue; }
    removed.push(name);
    await removeTree(doomed).catch(()=>{});
  }
  return {removed};
}
export class LinuxBrowserUser {
  constructor(root, identity, base = '/var/lib/chariox-browser', {owner=0, group=owner}={}) {
    this.uid=identity.uid; this.gid=identity.gid; this.base=base; this.owner=owner; this.group=group;
    this.kernelRoot=root;
    this.root=path.join(base,rootDigest(root));
  }
  async prepare() {
    try {
      await ownedBrowserDirectory(this.base,this.owner,this.group,0o711);
      await ownedBrowserDirectory(this.root,this.owner,this.group,0o711);
      await writeOwnerRecord(this.root,this.kernelRoot);
      this.home=await this.directory('home');
      return this;
    } catch (error) {
      if (error.code==='EPERM' || error.code==='EACCES') throw browserConfinementFailure(error.syscall==='fchown'?'ownershipCapability':undefined);
      throw browserUserFailure();
    }
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
  const user=await new LinuxBrowserUser(root,await account).prepare();
  // Once per kernel run, in the background: browser start never waits for it.
  reclaimed ??= sweepBrowserHomes(user.base).then(({removed})=>{
    if(removed.length)console.error(`[browser] reclaimed ${removed.length} browser dir(s) of deleted kernel homes`);
  },()=>{});
  return user;
}
