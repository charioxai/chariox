// MP-08/MP-09/MP-10/MP-11: account provisioning and the privilege boundary.
import test from 'node:test';
import assert from 'node:assert/strict';
import {ensureBrowserAccount,LinuxBrowserUser,browserUserFailure} from './linux-browser-user.mjs';
import {rootPolicyLaunch,rootPolicyNamespaceAvailable,launchEnvironment} from './kernel-browser-linux.mjs';
import {hostErrorLabel} from './kernel-browser-error-label.mjs';

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
 const launch=rootPolicyLaunch('/private/policy','/browser/runtime','/usr/bin/chrome',['--remote-debugging-pipe'],{uid:991,gid:990},'/kernel/private-namespace');
 assert.match(launch.command,/unshare$/);
 assert.equal(launch.args[0],'--mount');assert(!launch.args.includes('--user'));
 assert(launch.args[6].includes('exec /usr/bin/setpriv --reuid="$uid"'));
 assert(!launch.args.some(arg=>/--no-sandbox|--disable-setuid-sandbox/.test(arg)));
 assert(launch.args.includes('--remote-debugging-pipe'));
 assert.doesNotThrow(()=>launchEnvironment({},0));
 assert.throws(()=>rootPolicyLaunch('/a','/b','/c',[],{uid:0,gid:0}),/invalid browser user/);
});

test('MP-11 a host must support private mounts and actually use the dedicated uid',()=>{
 const user=new LinuxBrowserUser('/private/kernel',{uid:991,gid:990});
 let observed;
 assert.equal(rootPolicyNamespaceAvailable(user,(command,args,options)=>{observed={command,args,options};return {status:1}},'/installed/unshare'),false);
 assert.equal(observed.command,'/installed/unshare');
 assert(observed.args.includes('--reuid=991'));assert(observed.args.includes('--clear-groups'));
 assert.equal(observed.options.timeout,5000);
 assert.equal(rootPolicyNamespaceAvailable(user,()=>({status:0}),'/installed/unshare'),true);
});

test('MP-11 privileged mount construction never writes through the browser-owned temporary directory',()=>{
 const launch=rootPolicyLaunch('/kernel/policy','/browser/runtime','/usr/bin/chrome',[],{uid:991,gid:990},'/kernel/private-namespace');
 assert.match(launch.command,/unshare$/);
 assert(launch.args.includes('/kernel/private-namespace'));
 assert(launch.args[6].includes('new=$workspace/.root'));
 assert(!launch.args[6].includes('$temporary/.root'));
});
