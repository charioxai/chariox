import {acquireTerminationResource} from './termination.mjs';
import {registerTerminationCleanup} from './termination.mjs';
import {harness} from './paths.mjs';
import {clientModule,playwrightModule} from './paths.mjs';
// MP-08 / MP-10 / MP-11: real paired TUI; shape only its owned network namespace.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {execFileSync,execFile} from 'node:child_process';
import {promisify} from 'node:util';
const execAsync=promisify(execFile);
import net from 'node:net';
const {LocalIpcClient}=await import(clientModule('ipc.js'));
const {createCliRelayIdentityStore}=await import(clientModule('cli-relay-identity-store.js'));
const {parseArgs}=await import(clientModule('cli-options.js'));
const {CloudClient}=await import(clientModule('cloud-client.js'));
const {CloudClientCredentialStore}=await import(clientModule('cloud-client-credential-store.js'));
const {chromium}=await import(playwrightModule());
const {joinKernelTerminalPairingLink}=await import(clientModule('relay-api.js'));
import {spawnOwned,signalOwnedProcessGroup} from './owned.mjs';
import {lane,sleep} from './runtime.mjs';
export async function shapedRemoteTui(run,{useTui=true}={}){
 const root=run.root+'/ns';await mkdir(root,{recursive:true,mode:0o700});await writeFile(root+'/resolv.conf','nameserver 10.0.2.3\n');
 const identity=createCliRelayIdentityStore(run.root+'/home/relay/cli-identity-v1.json').getOrCreate();
 const pairing=(await run.client.send({CreateTerminalPairingLink:{terminal_type:'cli',alias:'ctxval-remote',expires_in_ms:15*60*1000}})).TerminalPairingLinkCreated.pairing;
 const options=parseArgs(['--terminal-pairing-link',pairing.pairing_link]);let bootstrap,holder,slirp,echo,cloud;const peers=new Set();
 async function stop(child){if(!child||child.exitCode!==null||child.signalCode!==null)return;signalOwnedProcessGroup(child,'SIGTERM');for(let i=0;i<50&&child.exitCode===null&&child.signalCode===null;i++)await sleep(100);if(child.exitCode===null&&child.signalCode===null)signalOwnedProcessGroup(child,'SIGKILL')}
 async function close(){cloud?.stop();await bootstrap?.close();for(const s of peers)s.destroy();if(echo)await new Promise(res=>echo.close(res));await stop(slirp);await stop(holder)}
 registerTerminationCleanup(close);
 try{
  let clientId=pairing.terminal_id,bootstrapToken=options.relayToken;
  if(bootstrapToken==='cloud-client-token-required'){
   if(!process.env.CTXVAL_ORIGIN)throw Error('Hosted CLIENT enrollment needs an origin');
   cloud=new CloudClient(new CloudClientCredentialStore(run.root+'/home/relay/cloud-client.json'),()=>identity);
   const browser=await acquireTerminationResource(()=>chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage']}),browser=>browser.close());
   try{const context=await browser.newContext(),page=await context.newPage();await page.goto(process.env.CTXVAL_ORIGIN+'/validation/ready');await cloud.login(process.env.CTXVAL_ORIGIN,async verification=>{if(new URL(verification.verificationUrl).origin!==process.env.CTXVAL_ORIGIN)throw Error('Wrong terminal approval origin');await page.goto(verification.verificationUrl);const approve=page.getByRole('button',{name:/approve|allow|connect/i});if(await approve.count()===1)await approve.click();});let fresh=false;for(let i=0;i<120;i++){const dashboard=await page.evaluate(async()=>{const r=await fetch('/dashboard');if(!r.ok)return null;const d=await r.json();return {targets:(d.relayTargets??[]).map(t=>({daemonId:t.daemonId,status:t.status}))}});if(dashboard?.targets.some(t=>t.daemonId===options.targetDaemonId&&t.status==='ONLINE')){fresh=true;break}await sleep(500)}await run.record('receiving-client-fresh-target',{target:options.targetDaemonId,fresh});if(!fresh)throw Error('Receiving client cannot issue against a stale/unowned target');}finally{await browser.close()}
   clientId='cli:'+identity.publicKeyThumbprint;bootstrapToken=(await cloud.issue(options.targetDaemonId)).token;
   await run.record('terminal-client-enrollment',{receivingClientOwnLogin:true,clientId,profileScopedToDisposableState:true});
  }
  const receivingCloud=Boolean(cloud);let joined;
  if(!receivingCloud){bootstrap=new LocalIpcClient(pairing.relay_url,{relayAuthToken:bootstrapToken,targetDaemonId:options.targetDaemonId,relayIdentity:identity});joined=await joinKernelTerminalPairingLink(bootstrap,pairing.pairing_link,clientId,identity.publicKeyThumbprint);await bootstrap.close();bootstrap=null;}
  holder=spawnOwned('unshare',['--net','--mount','--propagation','private','--','python3',harness+'/ns-holder.py',root],{env:run.env,stdio:'ignore',detached:true});let pid;
  for(let n=0;n<100;n++){try{pid=Number(await readFile(root+'/holder-ready','utf8'));if(Number.isInteger(pid)&&pid>1)break}catch{}await sleep(100)}assert(Number.isInteger(pid)&&pid>1);
  const nsnet='/proc/'+pid+'/ns/net',nsmount='/proc/'+pid+'/ns/mnt';const {readlink}=await import('node:fs/promises');assert.notEqual(await readlink(nsnet),await readlink('/proc/self/ns/net'));assert.notEqual(await readlink(nsmount),await readlink('/proc/self/ns/mnt'));
  slirp=spawnOwned(lane+'/ns-tools/usr/bin/slirp4netns',['--configure','--mtu=65520',String(pid),'tap0'],{env:{...run.env,LD_LIBRARY_PATH:lane+'/ns-tools/usr/lib/x86_64-linux-gnu'},stdio:'ignore',detached:true});
  for(let n=0;n<100;n++){try{const links=JSON.parse(execFileSync('nsenter',['--net='+nsnet,'ip','-j','link','show','tap0'],{stdio:['ignore','pipe','ignore']}).toString());if(links.length)break}catch{}await sleep(100)}
  for(const args of [ ['--net='+nsnet,'ip','link','set','lo','up'],['--mount='+nsmount,'mount','--bind',root+'/resolv.conf','/etc/resolv.conf'],['--net='+nsnet,'tc','qdisc','add','dev','tap0','root','handle','1:','tbf','rate','8mbit','burst','64kb','latency','2000ms'],['--net='+nsnet,'tc','qdisc','add','dev','tap0','parent','1:1','handle','10:','netem','delay','60ms','limit','4096']])execFileSync('nsenter',args,{stdio:'ignore'});
  echo=net.createServer(s=>{peers.add(s);s.on('error',()=>{});s.on('close',()=>peers.delete(s));s.pipe(s)});await new Promise(res=>echo.listen(0,'127.0.0.1',res));
  let calibration;for(let attempt=0;attempt<5;attempt++){calibration=JSON.parse((await execAsync('nsenter',['--net='+nsnet,'--mount='+nsmount,'python3','-c',`import socket,time,json\ns=socket.create_connection(('10.0.2.2',${echo.address().port}))\ndef ping(n):\n t=time.monotonic();s.sendall(bytes(n));got=0\n while got<n:got+=len(s.recv(min(65536,n-got)))\n return (time.monotonic()-t)*1000\nrtts=[ping(32)for _ in range(5)];mbps=2097152*8/ping(2097152)/1000\nprint(json.dumps({'rttMs':rtts,'uplinkMbps':mbps}))\ns.close()`],{timeout:20000})).stdout);
  await run.record('network-calibration-'+attempt,calibration);if(calibration.rttMs.every(x=>x>=60)&&calibration.uplinkMbps>=7&&calibration.uplinkMbps<=8.4)break;if(attempt===4)throw Error('Measured8Mbit/s60ms namespace calibration failed');await sleep(1000)}assert(calibration.rttMs.every(x=>x>=60));assert(calibration.uplinkMbps>=7&&calibration.uplinkMbps<=8.4);for(const s of peers)s.destroy();await new Promise(res=>echo.close(res));echo=null;
  const counters=JSON.parse(execFileSync('nsenter',['--net='+nsnet,'ip','-j','-s','link','show','tap0']).toString());
  function configure(pairing,joined){
   if(!useTui)return;
   if(receivingCloud){run.setRemote({clientId,prefix:['nsenter','--net='+nsnet,'--mount='+nsmount,'--'],connectionArgs:['--terminal-pairing-link',pairing.pairing_link],secret:pairing.pairing_link});return;}
   run.setRemote({clientId:joined.pairing.subject_id,prefix:['nsenter','--net='+nsnet,'--mount='+nsmount,'--'],connectionArgs:['--relay-url',pairing.relay_url,'--relay-token-env','CTXVAL_TERMINAL_TOKEN','--target-daemon-id',pairing.target_daemon_id],env:{CTXVAL_TERMINAL_TOKEN:joined.relay_token??(joined.kernel_pairing?bootstrapToken:undefined)},secret:joined.relay_token??bootstrapToken});
  }
  configure(pairing,joined);
  async function refresh(){if(!useTui)return;const next=(await run.client.send({CreateTerminalPairingLink:{terminal_type:'cli',alias:'ctxval-remote',expires_in_ms:15*60*1000}})).TerminalPairingLinkCreated.pairing;if(receivingCloud){configure(next);return;}const opts=parseArgs(['--terminal-pairing-link',next.pairing_link]);const c=new LocalIpcClient(next.relay_url,{relayAuthToken:opts.relayToken,targetDaemonId:opts.targetDaemonId,relayIdentity:identity});try{const current=await joinKernelTerminalPairingLink(c,next.pairing_link,clientId,identity.publicKeyThumbprint);configure(next,current)}finally{await c.close()}}
  run.setRemoteRefresher(refresh);
  await run.record('remote-tui-network',{relayUrl:pairing.relay_url,namespacePid:pid,namespaceOwned:true,calibration,counters});return {close,refresh,chromePrefix:['nsenter','--net='+nsnet,'--mount='+nsmount,'--'],async outage(ms=30000){assert(ms===30000);assert(Number.isInteger(pid)&&pid>1);const beforeRoutes=JSON.parse(execFileSync('nsenter',['--net='+nsnet,'ip','-j','route']).toString());const defaults=beforeRoutes.filter(r=>r.dst==='default');assert.equal(defaults.length,1);const route=defaults[0];assert.equal(route.dev,'tap0');assert(net.isIPv4(route.gateway));execFileSync('nsenter',['--net='+nsnet,'ip','link','set','tap0','down'],{stdio:'ignore'});await run.record('namespace-outage-down',{pid,ms,beforeRoutes});try{await sleep(ms)}finally{execFileSync('nsenter',['--net='+nsnet,'ip','link','set','tap0','up'],{stdio:'ignore'});const args=['--net='+nsnet,'ip','route','replace','default','via',route.gateway,'dev',route.dev];if(route.metric!==undefined){assert(Number.isInteger(route.metric)&&route.metric>=0);args.push('metric',String(route.metric))}execFileSync('nsenter',args,{stdio:'ignore'});const afterRoutes=JSON.parse(execFileSync('nsenter',['--net='+nsnet,'ip','-j','route']).toString());assert(afterRoutes.some(r=>r.dst==='default'&&r.gateway===route.gateway&&r.dev===route.dev));await run.record('namespace-outage-up',{pid,beforeRoutes,afterRoutes,defaultRouteRestored:true})}},async snapshot(){return {links:JSON.parse(execFileSync('nsenter',['--net='+nsnet,'ip','-j','-s','link','show','tap0']).toString())}}};
 }catch(e){await close();throw e}
}
