// MD-DISPLAY-02: shape only the viewer/proxy TCP leg in an owned namespace.
import { execFileSync } from 'node:child_process';
import { readlink } from 'node:fs/promises';
import { createServer, connect } from 'node:net';
export const profiles = {
 local: {rtt:0,jitter:0,loss:0,mbps:0},
 // MP-08/MP-10: supplementary owned-namespace 8Mbps link, not hosted acceptance.
 wan8: {rtt:80,jitter:5,loss:0,mbps:8},
 wan40: {rtt:40,jitter:2,loss:1,mbps:5},
 wan80: {rtt:80,jitter:5,loss:1,mbps:2},
 wan150: {rtt:150,jitter:10,loss:1,mbps:1},
 // MP-08/MP-10: the hosted campaign's shaped viewer leg (8.75 Mbit, +30 ms each way).
 hosted: {rtt:60,jitter:0,loss:0,mbps:8.75},
};
// MP-08/MP-10: supplementary userspace shaper for drills that need the host
// network (real public sites): per-direction FIFO serialized at the profile
// rate plus half the RTT. No qdisc or namespace changes on a shared host.
function shapedPipe(from,to,{delayMs,bytesPerMs}){
 let busyUntil=0,timer=null;const queue=[];
 const flush=()=>{timer=null;const now=performance.now();while(queue.length&&queue[0].at<=now)to.write(queue.shift().chunk);schedule();};
 const schedule=()=>{if(!timer&&queue.length)timer=setTimeout(flush,Math.max(0,queue[0].at-performance.now()));};
 from.on('data',chunk=>{const start=Math.max(performance.now(),busyUntil);busyUntil=start+chunk.length/bytesPerMs;queue.push({at:busyUntil+delayMs,chunk});schedule();});
 from.on('end',()=>{const wait=()=>queue.length?setTimeout(wait,5):to.end();wait();});
}
export async function shapeViewerLegUserspace(name, original) {
 const profile=profiles[name];
 if(!profile || name==='local')throw Error('MD-DISPLAY: invalid netem profile');
 const target=new URL(original),sockets=new Set(),shape={delayMs:profile.rtt/2,bytesPerMs:profile.mbps*1e6/8/1000};
 let failure;
 const proxy=createServer(down=>{
  const up=connect(Number(target.port),target.hostname);sockets.add(down);sockets.add(up);
  for(const socket of [down,up]){socket.setNoDelay(true);socket.on('error',error=>{failure=error;down.destroy();up.destroy()});socket.on('close',()=>sockets.delete(socket));}
  shapedPipe(down,up,shape);shapedPipe(up,down,shape);
 });
 await new Promise((resolve,reject)=>{proxy.once('error',reject);proxy.listen(0,'127.0.0.1',resolve)});
 return {url:`ws://127.0.0.1:${proxy.address().port}`,info:{name,...profile,userspace:true,note:'MP-08/MP-10: supplementary userspace FIFO shaper on the viewer/relay TCP leg only; unbounded queue like a deep netem limit.'},
  check(){if(failure)throw Error('MD-DISPLAY: owned TCP proxy failure')},
  async close(){for(const socket of sockets)socket.destroy();await new Promise(r=>proxy.close(r));return null},
 };
}
export async function shapeViewerLeg(name, original, hostNamespace) {
 const profile=profiles[name];
 if(!profile || name==='local')throw Error('MD-DISPLAY: invalid netem profile');
 const namespace=await readlink('/proc/self/ns/net');
 if(!hostNamespace || namespace===hostNamespace || namespace===await readlink('/proc/1/ns/net'))throw Error('MD-DISPLAY: refuse host namespace shaping');
 const ownedName=execFileSync('/usr/sbin/ip',['netns','identify',String(process.pid)],{encoding:'utf8'}).trim();
 if(!/^md-display-[0-9a-f]{12}$/.test(ownedName))throw Error('MD-DISPLAY: refuse unnamed or foreign namespace shaping');
 const target=new URL(original),sockets=new Set();
 let failure;
 const proxy=createServer(down=>{
  const up=connect(Number(target.port),target.hostname);sockets.add(down);sockets.add(up);
  for(const socket of [down,up]){
   socket.setNoDelay(true);socket.on('error',error=>{failure=error;down.destroy();up.destroy()});
   socket.on('close',()=>sockets.delete(socket));
  }
  down.pipe(up);up.pipe(down);
 });
 await new Promise((resolve,reject)=>{proxy.once('error',reject);proxy.listen(0,'127.0.0.1',resolve)});
 const port=proxy.address().port,commands=[];
 const tc=(...args)=>{commands.push(['tc',...args]);return execFileSync('/usr/sbin/tc',args,{encoding:'utf8'})};
 try {
  tc('qdisc','add','dev','lo','root','handle','1:','prio','bands','3','priomap',...Array(16).fill('0'));
  tc('qdisc','add','dev','lo','parent','1:3','handle','30:','netem','limit','4096','delay',`${profile.rtt/2}ms`,`${profile.jitter}ms`,'distribution','normal','loss',`${profile.loss}%`,'rate',`${profile.mbps}mbit`);
  for(const direction of ['sport','dport'])tc('filter','add','dev','lo','protocol','ip','parent','1:','prio','1','u32','match','ip','protocol','6','0xff','match','ip',direction,String(port),'0xffff','flowid','1:3');
 }catch(error){for(const socket of sockets)socket.destroy();await new Promise(r=>proxy.close(r));throw error}
 return {url:`ws://127.0.0.1:${port}`,info:{name,...profile,namespace,host_namespace:hostNamespace,proxy_port:port,commands,note:'MD-DISPLAY: bidirectional shared cap, only viewer/proxy TCP leg; kernel-to-relay, fixture HTTP and CDP control unshaped. Netem loss causes TCP retransmission, not dropped application frames.'},
  check(){if(failure)throw Error('MD-DISPLAY: owned TCP proxy failure')},
  async close(){const stats=JSON.parse(tc('-s','-j','qdisc','show','dev','lo'));for(const socket of sockets)socket.destroy();await new Promise(r=>proxy.close(r));tc('qdisc','del','dev','lo','root');return stats},
 };
}
