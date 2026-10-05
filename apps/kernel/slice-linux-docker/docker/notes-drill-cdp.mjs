// Native Notes drill only: private owner socket multiplexes its inherited CDP
// pipe to the fixture controller and stimulus. Never shipped as a host asset.
import net from 'node:net';
import { StringDecoder } from 'node:string_decoder';
import path from 'node:path';
import { chmod, mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { connectCdpPipe } from './kernel-browser-cdp-pipe.mjs';
export async function connectDrillCdp(root) {
 const socket=net.createConnection(await readFile(path.join(root,'drill-cdp-path'),'utf8'));
 await new Promise((ok,no)=>{socket.once('connect',ok);socket.once('error',no)});
 return connectCdpPipe(socket,socket);
}
export async function serveDrillCdp(root,connection) {
 const directory=await mkdtemp(path.join(tmpdir(),'mdn-'));
 await chmod(directory,0o700);
 const endpoint=path.join(directory,'cdp.sock');
 const peers=new Set();
 const server=net.createServer(socket=>{
  if(peers.size>=4){socket.destroy();return}peers.add(socket);
  let buffered='',pending=0;const decoder=new StringDecoder('utf8');
  const send=value=>{const wire=JSON.stringify(value)+'\0';if(socket.writableLength+Buffer.byteLength(wire)>16*1024*1024)socket.destroy();else socket.write(wire)};
  const off=connection.subscribe(send);
  socket.on('error',()=>{});
  socket.on('close',()=>{off();peers.delete(socket)});
  socket.on('data',chunk=>{
   buffered+=decoder.write(chunk);if(buffered.length>1024*1024){socket.destroy();return}
   let at;
   while((at=buffered.indexOf('\0'))>=0){
    const message=buffered.slice(0,at);buffered=buffered.slice(at+1);
    let request;try{request=JSON.parse(message)}catch{socket.destroy();return}
    if(!Number.isSafeInteger(request.id)||typeof request.method!=='string'||pending>=32){socket.destroy();return}
    pending++;
    void connection.send(request.method,request.params,request.sessionId).then(
     result=>send({id:request.id,result}),()=>send({id:request.id,error:{code:-32000,message:'Fixture CDP request failed'}})
    ).finally(()=>pending--);
   }
  });
 });
 await new Promise((ok,no)=>{server.once('error',no);server.listen(endpoint,ok)});
 await chmod(endpoint,0o600);
 await writeFile(path.join(root,'drill-cdp-path'),endpoint,{mode:0o600});
 let closed=false;
 return async()=>{if(closed)return;closed=true;for(const peer of peers)peer.destroy();await new Promise(ok=>server.close(ok));await rm(directory,{recursive:true,force:true})};
}
