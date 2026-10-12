import assert from 'node:assert/strict';
import test from 'node:test';
import { PassThrough } from 'node:stream';
import { connectCdpPipe } from './kernel-browser-cdp-pipe.mjs';
function pipe() {
 const input = new PassThrough(), output = new PassThrough();
 const connection = connectCdpPipe(input, output, 50);
 return { input, output, connection };
}
test('private CDP preserves sessions, fragmented replies and events', async () => {
 const {input,output,connection}=pipe();
 const events=[];connection.subscribe(event=>events.push(event));
 const pending=connection.send('Runtime.evaluate',{expression:'1'},'session');
 await Promise.resolve();
 const wire=JSON.parse(input.read().toString().slice(0,-1));
 assert.equal(wire.sessionId,'session');
 const reply=Buffer.from(JSON.stringify({id:wire.id,result:{value:'🙂'}})+'\0'+JSON.stringify({method:'Page.loadEventFired',sessionId:'session'})+'\0');
 for (const byte of reply) output.write(Buffer.from([byte]));
 assert.deepEqual(await pending,{value:'🙂'});
 assert.equal(events[0].method,'Page.loadEventFired');
 await connection.close();assert.equal(input.destroyed,true);assert.equal(output.destroyed,true);
});
for (const failure of ['end','invalid','oversized']) test(`private CDP fails closed on ${failure}`, async () => {
 const {output,connection}=pipe();
 const pending=connection.send('Browser.getVersion');
 const rejected=assert.rejects(pending);
 if(failure==='end') output.end();
 if(failure==='invalid') output.write('{bad}\0');
 if(failure==='oversized') output.write(Buffer.alloc(16*1024*1024+1,65));
 await rejected;assert.equal(connection.isOpen(),false);
 await connection.close();
});
test('private CDP bounds queued writes', async () => {
 const {input,connection}=pipe();
 input.write(Buffer.alloc(16*1024*1024,65));
 await assert.rejects(connection.send('Browser.getVersion'));
 assert.equal(connection.isOpen(),false);
 await connection.close();
});

// MP-08/MP-10/MP-11: never merge observations or their binding, only pipe writes.
test('MP-08/MP-10 concurrent native fences share a bounded ordered CDP write',async()=>{
 const {input,output,connection}=pipe();let writes=0;
 const write=input.write.bind(input);input.write=(...args)=>{writes++;return write(...args)};
 const frame=connection.send('Page.getFrameTree',{},'s'),visibility=connection.send('Runtime.evaluate',{expression:'document.visibilityState'},'s');
 try{
  await Promise.resolve();assert.equal(writes,1);
  const commands=input.read().toString().split('\0').filter(Boolean).map(JSON.parse);
  assert.deepEqual(commands.map(c=>c.method),['Page.getFrameTree','Runtime.evaluate']);
  for(const [i,c] of commands.entries())output.write(JSON.stringify({id:c.id,result:{binding:i}})+'\0');
  assert.deepEqual(await frame,{binding:0});assert.deepEqual(await visibility,{binding:1});
 }finally{await connection.close();await Promise.allSettled([frame,visibility])}
});
test('MP-11 closing a private CDP pipe retires queued commands before flushing',async()=>{
 const {input,connection}=pipe();let writes=0;input.write=()=>{writes++;return true};
 const pending=connection.send('Page.getFrameTree',{},'s');const rejected=assert.rejects(pending);
 await connection.close();await rejected;await Promise.resolve();assert.equal(writes,0);
});
test('MP-11 a synchronous batched CDP write failure rejects every fence',async()=>{
 const {input,output,connection}=pipe();
 input.write=()=>{throw Error('MP-11: injected private pipe failure')};
 const frame=connection.send('Page.getFrameTree',{},'s'),visibility=connection.send('Runtime.evaluate',{},'s');
 const rejected=Promise.all([assert.rejects(frame),assert.rejects(visibility)]);
 await rejected;assert.equal(connection.isOpen(),false);
 assert.equal(input.destroyed,true);assert.equal(output.destroyed,true);
});
