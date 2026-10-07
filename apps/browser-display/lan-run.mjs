// MD-DISPLAY-02/04: disposable LAN matrix; never manage any service or shared state.
import {execFileSync} from 'node:child_process';
import {mkdir,writeFile,readFile,access,chown,rmdir} from 'node:fs/promises';
import {constants} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {randomBytes} from 'node:crypto';
import {launchOwned,waitChild,signalChild,stopGroup} from './drill-owned-process.mjs';
import {memoryFloorGiB} from './drill-resources.mjs';
import {sourceIdentity} from './source-identity.mjs';
const kit=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
process.env.MD_KIT_MANIFEST=path.join(kit,'KIT_MANIFEST.json');
const identity=await sourceIdentity(),manifest=JSON.parse(await readFile(process.env.MD_KIT_MANIFEST,'utf8'));
if(process.getuid()!==0)throw Error('MD-DISPLAY: run with sudo for isolated netem namespaces');
const uid=Number(process.env.SUDO_UID||process.env.MD_LAN_UID),gid=Number(process.env.SUDO_GID||process.env.MD_LAN_GID);
if(!Number.isSafeInteger(uid)||uid<1||!Number.isSafeInteger(gid)||gid<1)throw Error('MD-DISPLAY: non-root invoking user required (sudo preserves SUDO_UID/GID)');
const supplementary=execFileSync('id',['-G',String(uid)],{encoding:'utf8'}).trim().split(/\s+/).map(Number);
if(!supplementary.length||!supplementary.every(n=>Number.isSafeInteger(n)&&n>=0))throw Error('MD-DISPLAY: invoking user group inventory unavailable');
// Only this sudo drill process changes its inherited groups, so sandboxed
// children have the invoking user's real render/video permissions. No account edits.
process.setgroups(supplementary);
const passwd=execFileSync('getent',['passwd',String(uid)],{encoding:'utf8'}).trim().split(':');
const operatorHome=process.env.MD_LAN_HOME||passwd[5];
if(!operatorHome||!path.isAbsolute(operatorHome))throw Error('MD-DISPLAY: invoking user home unavailable');
const run=path.join(operatorHome,'.chariox','dev','display-lan-'+new Date().toISOString().replaceAll(/[^0-9]/g,'')+'-'+randomBytes(4).toString('hex'));
await mkdir(run,{recursive:true,mode:0o755});await chown(run,uid,gid);
const state=path.join(run,'state'),evidence=path.join(run,'evidence');await mkdir(state,{mode:0o755});await mkdir(evidence);
const receipt={item:'MP-08/MP-10/MP-11 MD-DISPLAY-02/04',...identity,run,started_at:new Date().toISOString(),status:'RED_PREFLIGHT',cleanup:[],host:{arch:process.arch,node:process.version,kernel:execFileSync('uname',['-sr'],{encoding:'utf8'}).trim()},vaapi:{},runtime_user:{uid,gid,supplementary_groups:supplementary}};
let active,interrupted=false;
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{interrupted=true;void signalChild(active,signal).catch(()=>{})});
const child=async(command,args,env)=>{active=await launchOwned(command,args,{detached:true,stdio:'inherit',env});const exit=await waitChild(active);await stopGroup(active);active=null;return exit};
try{
 if(process.arch!=='x64')throw Error('MD-DISPLAY: Linux x86_64 kit required');
 for(const command of ['/usr/bin/Xvfb','/usr/sbin/ip','/usr/sbin/tc','/usr/sbin/ethtool','/usr/bin/ffmpeg'])await access(command,constants.X_OK);
 const chrome=process.env.MD_CHROME||['/usr/bin/google-chrome','/usr/bin/chromium','/usr/bin/chromium-browser'].find(p=>{try{execFileSync('test',['-x',p]);return true}catch{return false}});
 if(!chrome||!path.isAbsolute(chrome))throw Error('MD-DISPLAY: install Chromium/Chrome or supply MD_CHROME absolute path');
 receipt.host.chromium={path:chrome,version:execFileSync(chrome,['--version'],{encoding:'utf8'}).trim()};
 receipt.vaapi.device_present=await access('/dev/dri').then(()=>true,()=>false);
 receipt.vaapi.ffmpeg_encoders=execFileSync('/usr/bin/ffmpeg',['-hide_banner','-encoders'],{encoding:'utf8',stdio:['ignore','pipe','ignore']}).split('\n').filter(s=>s.includes('vaapi')).map(s=>s.trim());
 receipt.vaapi.note='Device/FFmpeg availability is not acceleration proof; motion_backend_vaapi in case traces records actual successful backend. Software fallback is retained.';
 const env={...process.env,MD_CHROME:chrome,MD_NODE:path.join(kit,'runtime/bin/node'),MD_KIT_MANIFEST:process.env.MD_KIT_MANIFEST,MD_SCRATCH_PARENT:state,MD_RUNTIME_UID:String(uid),MD_RUNTIME_GID:String(gid),MD_RUNTIME_PATH:path.join(kit,'runtime/bin')+':/usr/bin:/bin',MD_PYTHON:path.join(kit,'runtime/bin/python3'),MD_BINARY_LOADER:path.join(kit,'runtime/lib/ld-linux-x86-64.so.2'),MD_BINARY_LIBS:path.join(kit,'runtime/lib'),MD_SOURCE_ASSETS:'0',MD_NATIVE_WORKER:manifest.files.some(f=>f.path==='runtime/native-worker')?path.join(kit,'runtime/native-worker'):undefined,MD_GEOMETRY:process.env.MD_GEOMETRY||'1920x1080',MD_CODEC:process.env.MD_CODEC||'avc1.420033',MD_SOFTWARE:process.env.MD_SOFTWARE||'0',MD_WINDOW:'1',MD_CREDIT_WINDOW:process.env.MD_CREDIT_WINDOW||'3',MD_MEMORY_FLOOR_GIB:String(memoryFloorGiB(process.env.MD_MEMORY_FLOOR_GIB)),MD_CASES:process.env.MD_CASES||'local:docs:2000000,wan40:docs:2000000,wan80:docs:1000000,wan150:docs:500000,local:canvas:2000000,local:canvas:4000000,local:video:2000000,local:video:4000000,local:scroll:2000000,local:scroll:4000000,wan40:canvas:2000000,wan80:canvas:1000000,wan150:canvas:500000,local:canvas:16000000,local:video:16000000,local:scroll30:16000000,local:scroll60:32000000,local:wheel30:32000000'};
 receipt.settings={cases:env.MD_CASES,credit_window:Number(env.MD_CREDIT_WINDOW),native_geometry:env.MD_GEOMETRY==='1920x1080'?[1920,1080]:[2560,1600],dpr:env.MD_GEOMETRY==='1920x1080'?1:2,memory_floor_gib:Number(env.MD_MEMORY_FLOOR_GIB)};
 await writeFile(path.join(evidence,'binary.json'),JSON.stringify(manifest.binary,null,2));
 receipt.campaign=await child(path.join(kit,'runtime/bin/node'),[path.join(kit,'apps/browser-display/campaign.mjs'),path.join(kit,'runtime/kernel-tests'),evidence,path.join(kit,'tools'),path.join(kit,'pytools')],env);
 if(!interrupted){receipt.aggregate=await child(path.join(kit,'runtime/bin/python3'),[path.join(kit,'apps/browser-display/report.py'),evidence,run,kit],env);
  receipt.report=JSON.parse(await readFile(path.join(run,'report.json'),'utf8'));receipt.status=receipt.report.status;}
 else receipt.status='INTERRUPTED';
} catch(error){receipt.error=error.message;process.exitCode=1}
finally{
 try{await stopGroup(active)}catch(error){receipt.cleanup.push('RED owned child teardown: '+error.message);process.exitCode=1}
 // Remove only this freshly created run's EMPTY parent; drill keeps uncertain
 // teardown roots for inspection. Never recurse into preserved state on failure.
 try{await rmdir(state);receipt.cleanup.push('empty owned disposable state parent removed')}catch{receipt.cleanup.push('state parent retained: inspect case cleanup receipts');process.exitCode=1}
 receipt.finished_at=new Date().toISOString();
 process.exitCode=interrupted?130:process.exitCode||((receipt.campaign?.code===0&&receipt.aggregate?.code===0)?0:1);
 receipt.exit_code=process.exitCode;await writeFile(path.join(run,'RESULTS.json'),JSON.stringify(receipt,null,2));
 console.log(JSON.stringify({item:receipt.item,status:receipt.status,results:path.join(run,'RESULTS.json'),exit_code:receipt.exit_code}));
}
