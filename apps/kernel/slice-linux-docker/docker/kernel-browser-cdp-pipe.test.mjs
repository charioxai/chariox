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
