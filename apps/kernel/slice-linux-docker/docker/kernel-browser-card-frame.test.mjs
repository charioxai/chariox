import test from 'node:test';
import assert from 'node:assert/strict';
import {runInNewContext} from 'node:vm';
import {resolveCardFrame, restoreCardFrames} from './kernel-browser-card-frame.mjs';

test('restoring a card follows only one matching iframe on its original page origin', async () => {
  const identity = {page_origin:'https://example.com',frame_origin:'https://challenge.example',id:'',name:'',title:'Challenge'};
  let origin = identity.page_origin, matches = [], released = 0;
  const connection = {async send(method, params) {
    if (method === 'Runtime.callFunctionOn') {
      const document = {baseURI:origin+'/',querySelectorAll:() => matches};
      const node = runInNewContext(`(${params.functionDeclaration})`, {location:{origin}, document, URL})(params.arguments[0].value);
      return {result:node ? {objectId:'frame'} : {subtype:'null'}};
    }
    if (method === 'DOM.describeNode') return {node:{backendNodeId:81}};
    if (method === 'Runtime.releaseObject') released++;
  }};
  const frame = (title, src) => ({id:'',name:'',title,getAttribute:() => src});
  matches = [frame('Challenge','https://challenge.example/frame#new-render-id'),frame('Other','https://unrelated.example/')];
  assert.equal(await resolveCardFrame(connection,'session',1,identity),'backend:81');
  assert.equal(released,1);
  matches.push(frame('Challenge','https://challenge.example/another'));
  assert.equal(await resolveCardFrame(connection,'session',1,identity),null,'an ambiguous frame is never chosen');
  matches.pop(); origin = 'https://another.example';
  assert.equal(await resolveCardFrame(connection,'session',1,identity),null,'a different page cannot inherit the crop');
  assert.throws(() => restoreCardFrames([['card',{tab_id:'one',frame:{}}]]),/unreadable/);
});
