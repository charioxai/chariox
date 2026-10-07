import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
// Exercise the actual isolated observer admission function; no alternate guard.
const source=readFileSync(new URL('./kernel-browser-mirror-observer.mjs',import.meta.url),'utf8');
const activeSource=source.slice(source.indexOf('  const activeTarget ='),source.indexOf('  const focus ='));
function fixture({protectedRoot=false,ordinary=false,visible=true}={}){
 const document={},node={isConnected:true,localName:ordinary?'input':'body',ownerDocument:document};document.body=ordinary?{}:node;document.documentElement={};document.activeElement=node;
 const context={document,ids:new Map([[node,'n1']]),live:new Map([['n1',node]]),innerWidth:1280,innerHeight:800,box:()=>({x:0,y:visible?0:900,width:1280,height:182000}),unprotected:()=>{if(protectedRoot)throw Error('mirror protected input ancestor')},locate:()=>{throw Error('mirror offscreen node')}};
 return runInNewContext(activeSource+'activeTarget',context);
}
test('reverse Tab can dispatch from a visible long document body without pointer-centre admission',()=>assert.equal(fixture()([],false),'n1'));
test('document keyboard exception preserves protection, viewport and editable fences',()=>{
 assert.throws(()=>fixture({protectedRoot:true})([],false),/protected/);
 assert.throws(()=>fixture({visible:false})([],false),/offscreen/);
 assert.throws(()=>fixture()([],true),/text focus/);
 assert.throws(()=>fixture({ordinary:true})([],false),/offscreen/);
});
