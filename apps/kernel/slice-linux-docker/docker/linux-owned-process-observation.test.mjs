// MP-08/MP-10/MP-11: observation must never scan unrelated host processes.
import test from 'node:test';import assert from 'node:assert/strict';
import {observedDescendants} from './linux-owned-process.mjs';
const stat=(pid,parent,started)=>{const fields=Array(22).fill('0');fields[0]='S';fields[1]=String(parent);fields[19]=String(started);return `${pid} (owned) ${fields.join(' ')}`};
function fixture(){
 const files=new Map([['/proc/3100/stat',stat(3100,3000,10)],['/proc/3101/stat',stat(3101,3100,11)],['/proc/3102/stat',stat(3102,3101,12)],
 ['/proc/3100/task/3100/children',''],['/proc/3100/task/3110/children','3101'],['/proc/3101/task/3101/children','3102'],['/proc/3102/task/3102/children','']]);
 const tasks=new Map([['/proc/3100/task',['3100','3110']],['/proc/3101/task',['3101']],['/proc/3102/task',['3102']]]);
 const read=async p=>{if(!files.has(p))throw Object.assign(Error('gone'),{code:'ENOENT'});return files.get(p)};
 const list=async p=>{assert.notEqual(p,'/proc','observation scanned the entire host');return tasks.get(p)??[]};
 return {files,tasks,read,list,root:{pid:3100,parent:3000,started:'10'}};
}
test('MP-08/MP-10/MP-11 observer follows children created by any owned thread without a host scan',async()=>{
 const f=fixture();const rows=await observedDescendants([f.root],f);assert.deepEqual(rows.map(r=>r.pid),[3102,3101,3100]);
});

test('MP-11 observation refuses reused roots and invalid PID roots before child discovery',async()=>{
 for(const root of [{pid:0,parent:3000,started:'10'},{pid:1,parent:3000,started:'10'},{pid:3100,parent:3000,started:'old'}]){
  const f=fixture();assert.deepEqual(await observedDescendants([root],f),[]);
 }
});
test('MP-11 observation refuses child parent mismatch and tolerates disappearing tasks',async()=>{
 const f=fixture();f.files.set('/proc/3101/stat',stat(3101,9999,11));f.tasks.get('/proc/3100/task').push('gone');
 assert.deepEqual((await observedDescendants([f.root],f)).map(r=>r.pid),[3100]);
});
test('MP-11 observation rechecks parent identity before accepting listed children',async()=>{
 const f=fixture(),read=f.read;let seen=0;
 f.read=async p=>p==='/proc/3100/stat'&&++seen>=2?stat(3100,3000,20):read(p);
 assert.deepEqual(await observedDescendants([f.root],f),[]);
});
test('MP-11 observation accepts only a previously proved orphan root identity',async()=>{
 const f=fixture();f.files.set('/proc/3100/stat',stat(3100,1,10));
 assert.equal((await observedDescendants([f.root],f)).length,3);
 assert.deepEqual(await observedDescendants([{...f.root,started:'old'}],f),[]);
});
test('MP-11 observation fails closed when task discovery is denied or exceeds its bound',async()=>{
 const f=fixture();f.list=async()=>{throw Object.assign(Error('denied'),{code:'EACCES'})};await assert.rejects(observedDescendants([f.root],f),/denied/);
 const g=fixture();g.tasks.set('/proc/3100/task',Array.from({length:1025},(_,i)=>String(i+3100)));await assert.rejects(observedDescendants([g.root],g),/bound/);
});

test('MP-08/MP-10/MP-11 observation discovers a real child forked by a non-main thread and global cleanup settles it',async()=>{
 const {spawn}=await import('node:child_process'),{once}=await import('node:events');
 const {processIdentity,settleOwned,isOwnedAlive}=await import('./linux-owned-process.mjs');
 const script=`const {Worker}=require('node:worker_threads');const w=new Worker(\`const {spawn}=require('node:child_process');const c=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});require('node:worker_threads').parentPort.postMessage(c.pid);setInterval(()=>{},1000);\`,{eval:true});w.on('message',pid=>console.log(pid));setInterval(()=>{},1000);`;
 const child=spawn(process.execPath,['-e',script],{stdio:['ignore','pipe','ignore']});let root,known=[];
 try{
  root=await processIdentity(child.pid);assert(root);const [bytes]=await once(child.stdout,'data'),pid=Number(bytes.toString().trim());assert(pid>1);
  known=await observedDescendants([root]);assert(known.some(p=>p.pid===pid));assert(!known.some(p=>p.pid===process.pid));
 }finally{if(root)await settleOwned([{child,identity:root}],known)}
 for(const item of known)assert.equal(await isOwnedAlive(item),false);
});
