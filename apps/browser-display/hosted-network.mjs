// MP-08/MP-10/MP-11: owned-namespace shaping of real HTTPS/WSS TLS bytes.
// The upstream is the real Caddy endpoint; this bridge never terminates TLS.
import {createServer,isIP} from 'node:net';
import {execFileSync} from 'node:child_process';
import {readlink} from 'node:fs/promises';
import {launchOwned, signalChild, waitChild} from './drill-owned-process.mjs';
export function assertHostedNamespace(name,current,host) {
 if(!/^md-display-[0-9a-f]{12}$/.test(name)||current===host)throw Error('MP-11: refuse foreign or host namespace shaping');
}
// The Caddy upstream IP and its HTTPS host names are explicit configuration.
export function hostedTarget({upstream,hosts}={}) {
 if(!isIP(upstream??'')||!Array.isArray(hosts)||!hosts.length||!hosts.every(host=>/^[a-z0-9.-]{1,253}$/i.test(host??'')))throw Error('MP-11: hosted shaping requires MD_HOSTED_UPSTREAM and its HTTPS host names');
 return hosts.map(host=>`MAP ${host} 127.0.0.1`).join(', ');
}
export async function shapeHostedViewer({namespace,upstream,hosts,rtt=80,mbps=8}) {
 const resolverRules=hostedTarget({upstream,hosts});
 assertHostedNamespace(namespace,await readlink('/proc/self/ns/net'),await readlink('/proc/1/ns/net'));
 const actual=execFileSync('/usr/sbin/ip',['netns','identify',String(process.pid)],{encoding:'utf8'}).trim();
 if(actual!==namespace||rtt!==80||mbps!==8)throw Error('MP-11: hosted shaping ownership/target mismatch');
 const commands=[],tc=(...args)=>{commands.push(['tc',...args]);return execFileSync('/usr/sbin/tc',args,{encoding:'utf8'})};
 const children=new Set(),sockets=new Set();let failure,closed=false;
 const server=createServer(async socket=>{
  sockets.add(socket);let child;
  socket.on('error',()=>{});socket.once('close',()=>{sockets.delete(socket);if(child)void signalChild(child).catch(()=>{})});
  try{
   // The host-side process only copies encrypted TLS bytes over stdio.
   child=await launchOwned('/usr/bin/nsenter',['--net=/proc/1/ns/net',process.execPath,'--input-type=module','-e',
    `import{connect}from'node:net';const s=connect(443,'${upstream}');s.setNoDelay(true);process.stdin.pipe(s);s.pipe(process.stdout);s.on('error',()=>process.exitCode=1);s.on('close',()=>process.stdin.destroy());`],{stdio:['pipe','pipe','ignore']});
   children.add(child);socket.setNoDelay(true);socket.pipe(child.stdin);child.stdout.pipe(socket);
   child.stdin.on('error',()=>socket.destroy());child.stdout.on('error',()=>socket.destroy());
   if(socket.destroyed)await signalChild(child);
   await waitChild(child);children.delete(child);socket.destroy();
  }catch{if(!closed)failure=Error('MP-10: hosted TLS bridge failed');socket.destroy();if(child)await signalChild(child).catch(()=>{})}
 });
 try{
  await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(443,'127.0.0.1',resolve)});
  tc('qdisc','add','dev','lo','root','handle','1:','prio','bands','3','priomap',...Array(16).fill('0'));
  tc('qdisc','add','dev','lo','parent','1:3','handle','30:','netem','limit','256','delay','40ms','rate','8mbit');
  for(const dir of ['sport','dport'])tc('filter','add','dev','lo','protocol','ip','parent','1:','prio','1','u32','match','ip','protocol','6','0xff','match','ip',dir,'443','0xffff','flowid','1:3');
 }catch(error){for(const s of sockets)s.destroy();for(const c of children)await signalChild(c).catch(()=>{});server.close();throw error}
 return {
  info:{MP:'MP-08/MP-10/MP-11',namespace,rtt_ms:rtt,mbps,commands,tls:'end-to-end real Caddy TLS; no termination or packet inspection',limit:'Shaped b3 client, geographic WAN and real desktop acceptance remain separate'},
  resolverRules,
  check(){if(failure)throw failure},
  async close(){closed=true;const statistics=JSON.parse(tc('-s','-j','qdisc','show','dev','lo'));for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));for(const c of children)await signalChild(c);await Promise.all([...children].map(waitChild));tc('qdisc','del','dev','lo','root');return statistics},
 };
}
