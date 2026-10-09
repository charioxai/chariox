import test from 'node:test';
import assert from 'node:assert/strict';
import { readlink } from 'node:fs/promises';
import { shapeViewerLeg } from './drill-netem.mjs';
test('MD-DISPLAY netem refuses the host namespace and invalid profiles before mutation',async()=>{
 const current=await readlink('/proc/self/ns/net');
 await assert.rejects(shapeViewerLeg('wan40','ws://127.0.0.1:1',current),/refuse host namespace/);
 await assert.rejects(shapeViewerLeg('wan80','ws://127.0.0.1:1',undefined),/refuse host namespace/);
 await assert.rejects(shapeViewerLeg('wan40','ws://127.0.0.1:1','spoofed'),/refuse host namespace/);
 await assert.rejects(shapeViewerLeg('invalid','ws://127.0.0.1:1','different'),/invalid netem/);
});
test('MP-08/MP-10 userspace shaper adds the profile RTT and serializes at its rate',async()=>{
 const {createServer,connect}=await import('node:net');const {shapeViewerLegUserspace}=await import('./drill-netem.mjs');
 const echo=createServer(s=>s.pipe(s));await new Promise(r=>echo.listen(0,'127.0.0.1',r));
 const shaped=await shapeViewerLegUserspace('hosted',`ws://127.0.0.1:${echo.address().port}`);
 try{
  const socket=connect(Number(new URL(shaped.url).port),'127.0.0.1');await new Promise(r=>socket.once('connect',r));
  const roundTrip=async bytes=>{const start=performance.now();let got=0;await new Promise(r=>{socket.on('data',d=>{got+=d.length;if(got>=bytes){socket.removeAllListeners('data');r()}});socket.write(Buffer.alloc(bytes))});return performance.now()-start;};
  const small=await roundTrip(1);assert(small>=58&&small<200,`RTT ${small}`);
  const large=await roundTrip(100_000);assert(large>=60+100_000/(8.75e6/8/1000)-5,`rate ${large}`); // echo pipelines both directions
  socket.destroy();
 }finally{await shaped.close();await new Promise(r=>echo.close(r));}
});
