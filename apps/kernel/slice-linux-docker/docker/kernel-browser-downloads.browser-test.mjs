// MP-08/MP-09/MP-10/MP-11: real sandboxed Chrome through the production host request path.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {mkdir,readFile,access,rm} from 'node:fs/promises';
import path from 'node:path';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
const root=process.env.CHARIOX_BRDL_DRILL_ROOT;
assert(root,'explicit disposable drill root required');
const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function until(read,label){for(let i=0;i<200;i++){const value=await read();if(value)return value;await wait(50)}throw Error(`timed out: ${label}`)}

test('MP-08/MP-09/MP-10/MP-11 attachment navigation, link click, progress, readable path and cancellation', {timeout:60000},async()=>{
  await mkdir(root,{recursive:true});
  const host=new KernelBrowserHost(path.join(root,'browser'));
  const server=createServer((request,response)=>{
    if(request.url==='/')return response.end('<!doctype html><a style="display:block;width:300px;height:80px" href="/slow">Download slowly</a>');
    response.writeHead(200,{'Content-Disposition':'attachment; filename="report.bin"','Content-Length':4*1024*1024,'Content-Type':'application/octet-stream'});
    const interval=setInterval(()=>{if(response.destroyed){clearInterval(interval);return}response.write(Buffer.alloc(65536,65));if(response.bytesSent>=4*1024*1024){clearInterval(interval);response.end()}},40);
    let bytes=0;response.bytesSent=0;const write=response.write.bind(response);response.write=chunk=>{bytes+=chunk.length;response.bytesSent=bytes;return write(chunk)};
    response.on('close',()=>clearInterval(interval));
  });
  try{
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve)); const url=`http://127.0.0.1:${server.address().port}`;
    const initial=await host.request({op:'start'});assert.deepEqual(initial.downloads,[]);
    const opened=await host.request({op:'open',url:`${url}/`,_download_session:'room-a'});let tab=opened.tabs.find(tab=>tab.tab_id===opened.tab_id);
    const navigate=await host.request({op:'navigate',tab_id:tab.tab_id,generation:opened.generation,url:`${url}/attachment`,_download_session:'room-a'});
    assert.ok(Array.isArray(navigate.downloads),'attachment navigation succeeds and retains download state');
    await until(async()=>{const state=await host.request({op:'state'});return state.downloads.find(d=>d.state==='in_progress'&&d.bytes>0)},'observable progress');
    const done=await until(async()=>{const state=await host.request({op:'state'});return state.downloads.find(d=>d.state==='done')},'readable completed file');
    assert.equal((await readFile(done.path)).length,4*1024*1024);
    tab=(await host.request({op:'state'})).tabs.find(t=>t.tab_id===tab.tab_id);
    await host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'click',x:60,y:40},_download_session:'room-a'});
    const active=await until(async()=>{const state=await host.request({op:'state'});return state.downloads.find(d=>d.download_id!==done.download_id&&d.state==='in_progress')},'click download');
    await host.request({op:'cancel_download',tab_id:tab.tab_id,generation:opened.generation,download_id:active.download_id});
    const cancelled=await until(async()=>{const state=await host.request({op:'state'});return state.downloads.find(d=>d.download_id===active.download_id&&d.state==='cancelled')},'cancelled download');
    assert.equal(cancelled.path,null);
    const second=await host.request({op:'open',url:`${url}/`,_download_session:'room-b'});
    await host.request({op:'navigate',tab_id:second.tab_id,generation:second.generation,url:`${url}/other`,_download_session:'room-b'});
    const two=await until(async()=>{const state=await host.request({op:'state'});return state.downloads.some(d=>d.tab_id===second.tab_id)?state:null},'second tab download');
    assert.equal(new Set(two.downloads.map(d=>d.tab_id)).size,2);
    await host.handle({id:1,method:'host.downloads.cleanup',params:{session_id:'room-a'}});
    await assert.rejects(access(done.path));assert.equal(host.downloads.state().some(d=>d.tab_id===tab.tab_id),false);
    console.log(JSON.stringify({result:'PASS',attachment:done,link:cancelled,two_tabs:true,cleanup:true}));
  }finally{await host.stop();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));await rm(root,{recursive:true,force:true})}
});
