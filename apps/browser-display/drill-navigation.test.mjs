import test from 'node:test';import assert from 'node:assert/strict';
import {assertIndependentNavigation} from './drill-navigation.mjs';
test('MP-08/MP-10/MP-11: protected codec rejection permits an independent PNG navigation fallback',()=>{
 assert.doesNotThrow(()=>assertIndependentNavigation({document_id:'new',kind:'png'},'old','avc1.420033'));
 assert.throws(()=>assertIndependentNavigation({document_id:'old',kind:'png'},'old','avc1.420033'));
 assert.throws(()=>assertIndependentNavigation({document_id:'new',kind:'tiles'},'old','avc1.420033'));
});
test('MD-DISPLAY reviewer04:53 PNG-only navigation uses negotiated independent PNG',()=>{
 assert.doesNotThrow(()=>assertIndependentNavigation({document_id:'new',kind:'png'},'old','png'));
 assert.doesNotThrow(()=>assertIndependentNavigation({document_id:'new',kind:'video',key:true},'old','vp09.00.10.08'));
 assert.throws(()=>assertIndependentNavigation({document_id:'old',kind:'png'},'old','png'));
 assert.throws(()=>assertIndependentNavigation({document_id:'new',kind:'tiles'},'old','png'));
 assert.throws(()=>assertIndependentNavigation({document_id:'new',kind:'video',key:false},'old','vp09.00.10.08'));
});
