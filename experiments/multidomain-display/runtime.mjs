// MD-DISPLAY-02/03: lane-owned processes and resource sampling only.
import { execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, writeFile, readdir, rm, chmod, chown, statfs } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import {launchOwned,checkChild,rememberGroup,stopGroup} from './owned-process.mjs';
let failure;
export function failRun(error){failure??=error instanceof Error?error:Error(String(error))}
export function throwIfFailed(){if(failure)throw failure}
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let stopping=false;
export function requestStop(){stopping=true}
export async function until(check, label, timeout = 15000) {
  const end = performance.now() + timeout;
  while (performance.now() < end) { throwIfFailed();if(stopping)throw Error('MD-DISPLAY interrupted; settle owned runtime');const v = await check(); if (v) return v; await pause(20); }
  throw Error(`MD-DISPLAY timeout: ${label}`);
}
export async function resources(groups = []) {
  const mem = await readFile('/proc/meminfo','utf8'), disk = await statfs('/');
  const available = Number(mem.match(/^MemAvailable:\s+(\d+)/m)[1]) * 1024;
  const processes = [];
  for (const name of await readdir('/proc')) {
    if (!/^\d+$/.test(name)) continue;
    try {
      const text = await readFile(`/proc/${name}/stat`,'utf8'), fields = text.slice(text.lastIndexOf(')') + 2).split(' ');
      if (groups.includes(Number(fields[2]))||Number(name)===process.pid) processes.push({ pid: Number(name), group: Number(fields[2]), start_ticks: fields[19], state: fields[0], cpu_ticks: Number(fields[11])+Number(fields[12]), rss_bytes: Number(fields[21])*4096 });
    } catch {}
  }
  return { at: new Date().toISOString(), monotonic_ms: performance.now(), mem_available_bytes: available, disk_free_bytes: Number(disk.bavail)*Number(disk.bsize), processes, rss_bytes: processes.reduce((n,p)=>n+p.rss_bytes,0), cpu_ticks: processes.reduce((n,p)=>n+p.cpu_ticks,0) };
}
export function guard(sample) {
  if (sample.mem_available_bytes < 16 * 1024**3 || sample.disk_free_bytes < 10 * 1024**3) throw Error('MD-DISPLAY resource floor reached');
}
export async function display({executable='Xvfb',args=['-displayfd','3','-screen','0','1920x1200x24','-nolisten','tcp','-ac'],timeout=15000}={}) {
  let child;
  try {
    child=await launchOwned(executable,args,{stdio:['ignore','ignore','ignore','pipe'],detached:true});
    let number='';child.stdio[3].on('data',b=>number+=b.toString());
    await until(()=>{checkChild(child,'Xvfb');return number.includes('\n')},'Xvfb displayfd',timeout);
    return {child,number:`:${number.trim()}`};
  } catch(error){await stopGroup(child);throw error}
}
export async function headedBrowser(chromium, output, name, screen, sandbox = true) {
  const scratch = await mkdtemp(path.join(tmpdir(),`chariox-md-display-${name}-`));
  let child,browser;const log=[];
  const settle=async()=>{try{await stopGroup(child)}finally{try{await writeFile(path.join(output,`${name}-launch.log`),log.join(''))}finally{await rm(scratch,{recursive:true,force:true})}}};
  try {
    await chmod(scratch,0o700); await chown(scratch,65534,65534);
    const home = path.join(scratch,'home'); await mkdir(home,{mode:0o700}); await chown(home,65534,65534);
    const flags=['--remote-debugging-address=127.0.0.1','--remote-debugging-port=0',`--user-data-dir=${home}`,'--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--disable-dev-shm-usage','--window-size=1920,1200','--force-device-scale-factor=2','--disable-background-timer-throttling','--disable-renderer-backgrounding','about:blank'];
    if (!sandbox) flags.push('--no-sandbox');
    child=await launchOwned(process.env.MD_CHROME || '/usr/bin/google-chrome',flags,{cwd:scratch,uid:65534,gid:65534,detached:true,env:{HOME:home,PATH:'/usr/bin:/bin',DISPLAY:screen,TMPDIR:scratch,XDG_RUNTIME_DIR:scratch},stdio:['ignore','ignore','pipe']});
    child.stderr.on('data',b=>log.push(b.toString()));
    const port=await until(async()=>{checkChild(child,`${name} Chrome`);try{return Number((await readFile(path.join(home,'DevToolsActivePort'),'utf8')).split('\n')[0])}catch{return null}},`${name} debugging port`);
    browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    return { child,browser,scratch,flags,sandbox, async close(){await rememberGroup(child);try{await browser.close()}finally{await settle()}} };
  } catch (error) {
    await settle();throw error;
  }
}
export const git = (...args) => execFileSync('git',args,{encoding:'utf8'}).trim();
export { pause, stopGroup };
