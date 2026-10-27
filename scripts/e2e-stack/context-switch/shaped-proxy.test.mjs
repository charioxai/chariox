// MP-08 / MP-10 / MP-11: missing worker must fail before any live acceptance run.
import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import {once} from 'node:events';
import {startShapedProxy} from './shaped-proxy.mjs';
test('worker forwards opaque bytes with delay, refuses foreign authorities, closes', {timeout:10000},async()=>{
 const peers=new Set(),echo=net.createServer(s=>{peers.add(s);s.on('close',()=>peers.delete(s));s.on('error',()=>{});s.pipe(s);});
 echo.listen(0,'127.0.0.1');await once(echo,'listening');
 let proxy,socket;
 try{
  const authority='127.0.0.1:'+echo.address().port;
  proxy=await startShapedProxy({targets:[authority]});
  socket=net.connect(new URL(proxy.url).port,'127.0.0.1');await once(socket,'connect');
  let data=Buffer.alloc(0);socket.on('data',b=>data=Buffer.concat([data,b]));
  socket.write('CONNECT '+authority+' HTTP/1.1\r\nHost: '+authority+'\r\n\r\n');
  const wait=async fn=>{const end=Date.now()+3000;while(!fn()){if(Date.now()>end)throw Error('Tunnel timeout');await new Promise(r=>setTimeout(r,5));}};
  await wait(()=>data.includes('\r\n\r\n'));assert.match(data.toString(),/^HTTP\/1.1 200/);data=Buffer.alloc(0);
  const payload=Buffer.from([0,255,19,0,42]),start=performance.now();socket.write(payload);
  await wait(()=>data.length===payload.length);assert.deepEqual(data,payload);assert(performance.now()-start>=60);
  const stats=await proxy.sampleStats();assert.equal(stats.upBytes,payload.length);assert.equal(stats.downBytes,payload.length);assert.equal(stats.failures,0);
  socket.destroy();socket=net.connect(new URL(proxy.url).port,'127.0.0.1');await once(socket,'connect');
  const reply=once(socket,'data');socket.write('CONNECT foreign.invalid:443 HTTP/1.1\r\n\r\n');assert.match((await reply)[0].toString(),/^HTTP\/1.1 403/);
 }finally{socket?.destroy();await proxy?.close();for(const s of peers)s.destroy();await new Promise(r=>echo.close(r));}
});
