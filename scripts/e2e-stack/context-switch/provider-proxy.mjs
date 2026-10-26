// MP-08 / MP-10 / MP-11: provider-only real CONNECT transport outage; no TLS payload inspection.
import net from 'node:net';
import {writeFile} from 'node:fs/promises';
export async function providerNetwork({root,env,record,provider='codex'}){
 if(!['codex','claude'].includes(provider))throw Error('Unsupported isolated official provider');const executable=provider==='claude'?env.CHARIOX_CLAUDE_BIN:'/usr/local/bin/codex';
 if(typeof executable!=='string'||!executable)throw Error('Official provider executable required');const quotedExecutable="'"+executable.replaceAll("'","'\\''")+"'";
 const sockets=new Set(),tunnels=new Set(),hosts=new Set();let blocked=false,totalConnections=0;
 const server=net.createServer(client=>{
  sockets.add(client);client.on('error',()=>{});client.on('close',()=>sockets.delete(client));let header=Buffer.alloc(0);
  function receive(chunk){header=Buffer.concat([header,chunk]);if(header.length>16384){client.destroy();return}const split=header.indexOf('\r\n\r\n');if(split<0)return;client.removeListener('data',receive);const first=header.subarray(0,split).toString().split('\r\n')[0];const match=/^CONNECT ([A-Za-z0-9.-]+):(\d+) HTTP\/1\.[01]$/.exec(first);if(!match||Number(match[2])!==443){client.destroy();return}hosts.add(match[1]);totalConnections++;if(blocked)return;const upstream=net.connect({host:match[1],port:443});sockets.add(upstream);tunnels.add(client);tunnels.add(upstream);upstream.on('close',()=>{sockets.delete(upstream);tunnels.delete(upstream);tunnels.delete(client)});upstream.on('error',()=>client.destroy());client.on('close',()=>upstream.destroy());upstream.on('connect',()=>{client.write('HTTP/1.1 200 Connection Established\r\n\r\n');const extra=header.subarray(split+4);if(extra.length)upstream.write(extra);client.pipe(upstream);upstream.pipe(client)});
  }
  client.on('data',receive);
 });
 await new Promise((res,rej)=>{server.once('error',rej);server.listen(0,'127.0.0.1',res)});const url='http://127.0.0.1:'+server.address().port,wrapper=root+'/'+provider+'-egress.sh';await writeFile(wrapper,'#!/bin/sh\nexport HTTPS_PROXY='+url+' HTTP_PROXY='+url+' ALL_PROXY='+url+' NO_PROXY=localhost,127.0.0.1,::1\nexec '+quotedExecutable+' "$@"\n',{mode:0o700});if(provider==='claude')env.CHARIOX_CLAUDE_BIN=wrapper;else env.CHARIOX_CODEX_BIN=wrapper;
 await record('provider-proxy',{providerExecutable:executable,provider,proxyPort:server.address().port,localKernelControlUnaffected:true,tlsPayloadInspected:false});
 return {async close(){for(const s of sockets)s.destroy();await new Promise(res=>server.close(res))},async outage(ms=130000){if(![130000,310000].includes(ms)||!totalConnections)throw Error('Real official provider proxy egress not established');blocked=true;for(const s of tunnels)s.destroy();await record('provider-outage-down',{ms,observedConnections:totalConnections,hosts:[...hosts],scope:'only the owned official '+provider+' HTTP CONNECT proxy'});try{await new Promise(res=>setTimeout(res,ms))}finally{blocked=false;for(const s of sockets)if(!tunnels.has(s))s.destroy();await record('provider-outage-up',{observedConnections:totalConnections,hosts:[...hosts],scope:'owned provider proxy restored'})}}};
}
