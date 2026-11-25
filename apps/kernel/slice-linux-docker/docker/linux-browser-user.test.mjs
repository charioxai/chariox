// MP-08/MP-09/MP-10/MP-11: account provisioning and the privilege boundary.
import test from 'node:test';
import assert from 'node:assert/strict';
import {ensureBrowserAccount,LinuxBrowserUser,browserUserFailure} from './linux-browser-user.mjs';
import {rootPolicyLaunch,requireRootPolicyNamespace,launchEnvironment} from './kernel-browser-linux.mjs';
import {hostErrorLabel,hostFailureMessage} from './kernel-browser-error-label.mjs';

test('MP-08 root provisions a locked system account and tolerates another kernel winning creation',async()=>{
 const calls=[];let lookups=0;
 const identity=await ensureBrowserAccount(async(command,args)=>{
  calls.push([command,args]);
  if(command.endsWith('getent')){if(!lookups++)throw Error('missing');return {stdout:'chariox-browser:x:991:990::/nonexistent:/usr/sbin/nologin\n'};}
  throw Error('another kernel created it');
 });
 assert.deepEqual(identity,{uid:991,gid:990});
 assert(calls[1][1].includes('--system'));assert(calls[1][1].includes('--no-create-home'));
});
test('MP-11 an unavailable or root account fails with recovery and a typed public reason',async()=>{
 for(const entry of ['', 'chariox-browser:x:0:0::/root:/bin/bash','chariox-browser:x:991:990::/home/browser:/bin/bash']){
  await assert.rejects(ensureBrowserAccount(async()=>({stdout:entry})),error=>error.code==='browser_user_unavailable'&&error.message.includes('retry'));
 }
 assert.match(hostErrorLabel(browserUserFailure()),/browser_user_unavailable/);
});
test('MP-11 root browser environment has its own home and no inherited secrets or display',()=>{
 const user=new LinuxBrowserUser('/private/kernel',{uid:991,gid:990});user.home='/browser/home';
 const env=user.environment({PATH:'/bin:/usr/bin',HOME:'/root',DISPLAY:':0',XAUTHORITY:'/root/auth',CLAUDE_CODE_OAUTH_TOKEN:'synthetic',CHARIOX_KERNEL_BROWSER_HEADLESS:'1'});
 assert.deepEqual(env,{PATH:'/bin:/usr/bin',CHARIOX_KERNEL_BROWSER_HEADLESS:'1',HOME:'/browser/home',USER:'chariox-browser',LOGNAME:'chariox-browser'});
 const command=user.command('/usr/bin/Xvfb',['-nolisten','tcp']);
 assert.equal(command.binary,'/usr/bin/setpriv');assert(command.args.includes('--clear-groups'));
 assert(command.args.includes('--reuid=991'));
});
test('MP-08 root owns policy mounts and drops uid before sandboxed Chrome inherits CDP pipes',()=>{
 const launch=rootPolicyLaunch('/private/policy','/browser/runtime','/usr/bin/chrome',['--remote-debugging-pipe'],{uid:991,gid:990},'/kernel/private-namespace',undefined,'/fixture/app-wasm-state-test');
 assert.match(launch.command,/unshare$/);
 assert.equal(launch.args[0],'--mount');assert(!launch.args.includes('--user'));
 assert(launch.args[6].includes('exec /usr/bin/setpriv --reuid="$uid"'));
 assert(!launch.args.some(arg=>/--no-sandbox|--disable-setuid-sandbox/.test(arg)));
 assert(launch.args.includes('--remote-debugging-pipe'));
 assert.doesNotThrow(()=>launchEnvironment({},0));
 assert.throws(()=>rootPolicyLaunch('/a','/b','/c',[],{uid:0,gid:0},undefined,undefined,'/fixture/app-wasm-state-test'),/invalid browser user/);
});

test('MP-11 a host must support private mounts and actually use the dedicated uid',()=>{
 const user=new LinuxBrowserUser('/private/kernel',{uid:991,gid:990});
 let observed;
 assert.throws(()=>requireRootPolicyNamespace(user,(command,args,options)=>{observed={command,args,options};return {status:1}},'/installed/unshare'),{code:'browser_user_unavailable'});
 assert.equal(observed.command,'/installed/unshare');
 assert(observed.args.includes('--reuid=991'));assert(observed.args.includes('--clear-groups'));
 assert.equal(observed.options.timeout,5000);
 assert.doesNotThrow(()=>requireRootPolicyNamespace(user,()=>({status:0}),'/installed/unshare'));
});

test('N4 a denied root browser namespace gives host recovery guidance',()=>{
 const user=new LinuxBrowserUser('/fixture/kernel',{uid:991,gid:990});
 assert.throws(()=>requireRootPolicyNamespace(user,()=>({status:1,stderr:'unshare: Operation not permitted private-fixture'}),'/installed/unshare'),error=>{
  const message=hostFailureMessage(error);
  assert.equal(error.code,'browser_user_unavailable');
  assert.match(message,/nested confinement|AppArmor|CAP_SYS_ADMIN/);
  assert.match(message,/host service|host operator/);
  assert(!message.includes('private-fixture'));
  return true;
 });
});

test('MP-11 privileged mount construction never writes through the browser-owned temporary directory',()=>{
 const launch=rootPolicyLaunch('/kernel/policy','/browser/runtime','/usr/bin/chrome',[],{uid:991,gid:990},'/kernel/private-namespace',undefined,'/fixture/app-wasm-state-test');
 assert.match(launch.command,/unshare$/);
 assert(launch.args.includes('/kernel/private-namespace'));
 assert(launch.args[6].includes('new=$workspace/.root'));
 assert(!launch.args[6].includes('$temporary/.root'));
});

test('MP-11 browser namespace excludes the configured WASM App data root', () => {
 const storage='/home/owner/.chariox/state/app-wasm-state-database';
 const launch=rootPolicyLaunch('/kernel/policy','/browser/runtime','/usr/bin/chrome',[],{uid:991,gid:990},'/kernel/private-namespace','/usr/bin/unshare',storage);
 assert.ok(launch.args.includes(storage));
 assert.ok(!launch.args.includes('/var/lib/chariox-app-storage'));
});

// Row 61: a browser account dir is reclaimed once its kernel's root is gone, never on a guess.
import {mkdtemp,mkdir as mkdirP,writeFile,readFile,symlink,rm,stat,readdir,chown} from 'node:fs/promises';
import os from 'node:os';
import nodePath from 'node:path';
import {createHash as hash} from 'node:crypto';
import {sweepBrowserHomes} from './linux-browser-user.mjs';
const me={uid:process.getuid(),gid:process.getgid()};
async function reclaimFixture(){
 const top=await mkdtemp(nodePath.join(os.tmpdir(),'browser-reclaim-'));
 const base=nodePath.join(top,'chariox-browser');
 const kernel=async name=>{const root=nodePath.join(top,name,'state','kernel-browser','k');await mkdirP(root,{recursive:true});const user=await new LinuxBrowserUser(root,me,base,{owner:me.uid,group:me.gid}).prepare();return {root,user,dir:user.root};};
 return {top,base,kernel};
}
const exists=file=>stat(file).then(()=>true,()=>false);

test('row 61 U1 a deleted kernel home is reclaimed at the next start; a live one is kept',async()=>{
 const {top,base,kernel}=await reclaimFixture();
 try{
  const a=await kernel('home-a'),b=await kernel('home-b');
  const record=JSON.parse(await readFile(nodePath.join(a.dir,'owner.json'),'utf8'));
  assert.equal(record.root,a.root);
  assert.equal((await stat(nodePath.join(a.dir,'owner.json'))).mode&0o777,0o600);
  await rm(nodePath.join(top,'home-a'),{recursive:true});
  const result=await sweepBrowserHomes(base,{owner:me.uid});
  assert.equal(await exists(a.dir),false);
  assert.equal(await exists(b.dir),true);
  assert.deepEqual(result.removed,[nodePath.basename(a.dir)]);
 }finally{await rm(top,{recursive:true,force:true});}
});

test('row 61 U2 a live home keeps its profile across stop and start',async()=>{
 const {top,base,kernel}=await reclaimFixture();
 try{
  const a=await kernel('home-a');
  const profile=await a.user.directory('profile');
  await writeFile(nodePath.join(profile,'Cookies'),'session');
  await sweepBrowserHomes(base,{owner:me.uid});
  const again=await new LinuxBrowserUser(a.root,me,base,{owner:me.uid,group:me.gid}).prepare();
  assert.equal(again.root,a.dir);
  assert.equal(await readFile(nodePath.join(await again.directory('profile'),'Cookies'),'utf8'),'session');
  assert.equal(JSON.parse(await readFile(nodePath.join(a.dir,'owner.json'),'utf8')).root,a.root);
 }finally{await rm(top,{recursive:true,force:true});}
});

test('row 61 U3 a missing, forged or foreign record never removes a dir',async t=>{
 const {top,base,kernel}=await reclaimFixture();
 try{
  const live=await kernel('home-live');
  const gone=nodePath.join(top,'gone','state');
  const named=root=>nodePath.join(base,hash('sha256').update(root).digest('hex'));
  // 1. legacy dir without a record
  const legacy=named(nodePath.join(top,'legacy'));await mkdirP(legacy);
  // 2. record naming a missing root that does not hash to this dir
  const forged=named(nodePath.join(top,'forged'));await mkdirP(forged);
  await writeFile(nodePath.join(forged,'owner.json'),JSON.stringify({version:1,root:gone}),{mode:0o600});
  // 3. record that is a symlink to a real record of a missing root
  const linked=named(gone);await mkdirP(linked);
  await writeFile(nodePath.join(top,'real-record.json'),JSON.stringify({version:1,root:gone}),{mode:0o600});
  await symlink(nodePath.join(top,'real-record.json'),nodePath.join(linked,'owner.json'));
  // 4. a valid-looking record owned by another account
  const goneB=nodePath.join(top,'gone-b','state');const foreign=named(goneB);await mkdirP(foreign);
  await writeFile(nodePath.join(foreign,'owner.json'),JSON.stringify({version:1,root:goneB}),{mode:0o600});
  // Needs CAP_CHOWN to hand the record to nobody; without it this case is reported as skipped.
  const foreignChecked=await chown(nodePath.join(foreign,'owner.json'),65534,65534).then(()=>true,()=>false);
  if(!foreignChecked){t.diagnostic('foreign-owner case skipped: no CAP_CHOWN');await rm(foreign,{recursive:true});}
  // 5. an entry that is itself a symlink to a dir holding a valid record of a missing root
  const goneC=nodePath.join(top,'gone-c','state');const target=nodePath.join(top,'elsewhere');await mkdirP(target);
  await writeFile(nodePath.join(target,'owner.json'),JSON.stringify({version:1,root:goneC}),{mode:0o600});
  await symlink(target,named(goneC));
  // 6. a valid record of a missing root whose browser is still running (moved home)
  const goneD=nodePath.join(top,'gone-d','state');const running=named(goneD);await mkdirP(nodePath.join(running,'profile'),{recursive:true});
  await writeFile(nodePath.join(running,'owner.json'),JSON.stringify({version:1,root:goneD}),{mode:0o600});
  await symlink(`fixture-host-${process.pid}`,nodePath.join(running,'profile','SingletonLock'));
  const result=await sweepBrowserHomes(base,{owner:me.uid});
  for(const dir of [live.dir,legacy,forged,linked,...foreignChecked?[foreign]:[],named(goneC),target,running])assert.equal(await exists(dir),true,dir);
  assert.equal(await exists(nodePath.join(target,'owner.json')),true);
  assert.deepEqual(result.removed,[]);
 }finally{await rm(top,{recursive:true,force:true});}
});

test('row 61 U4 reclaiming never follows a symlink inside the browser dir',async()=>{
 const {top,base,kernel}=await reclaimFixture();
 try{
  const a=await kernel('home-a');
  const outside=nodePath.join(top,'outside');await mkdirP(nodePath.join(outside,'tree'),{recursive:true});
  await writeFile(nodePath.join(outside,'keep.txt'),'keep');await writeFile(nodePath.join(outside,'tree','keep.txt'),'keep');
  const profile=await a.user.directory('profile');
  await symlink(nodePath.join(outside,'keep.txt'),nodePath.join(profile,'link'));
  await symlink(nodePath.join(outside,'tree'),nodePath.join(profile,'dirlink'));
  await rm(nodePath.join(top,'home-a'),{recursive:true});
  await sweepBrowserHomes(base,{owner:me.uid});
  assert.equal(await exists(a.dir),false);
  assert.equal(await readFile(nodePath.join(outside,'keep.txt'),'utf8'),'keep');
  assert.equal(await readFile(nodePath.join(outside,'tree','keep.txt'),'utf8'),'keep');
  assert.deepEqual((await readdir(base)).filter(name=>name.startsWith('.')),[]);
 }finally{await rm(top,{recursive:true,force:true});}
});
