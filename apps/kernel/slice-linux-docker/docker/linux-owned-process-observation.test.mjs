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
