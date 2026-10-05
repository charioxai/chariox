import test from 'node:test';import assert from 'node:assert/strict';
import {assertIndependentNavigation} from './drill-navigation.mjs';
test('MD-DISPLAY reviewer04:53 PNG-only navigation uses negotiated independent PNG',()=>{
 assert.doesNotThrow(()=>assertIndependentNavigation({document_id:'new',kind:'png'},'old','png'));
 assert.doesNotThrow(()=>assertIndependentNavigation({document_id:'new',kind:'video',key:true},'old','vp09.00.10.08'));
 assert.throws(()=>assertIndependentNavigation({document_id:'old',kind:'png'},'old','png'));
 assert.throws(()=>assertIndependentNavigation({document_id:'new',kind:'tiles'},'old','png'));
 assert.throws(()=>assertIndependentNavigation({document_id:'new',kind:'video',key:false},'old','vp09.00.10.08'));
});
