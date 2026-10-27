// MP-08/MP-10/MP-11 supplementary bounds; real FontFace lifecycle is browser-tested.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {nativeFontRequirements} from './kernel-browser-mirror2-fonts.mjs';
const hash=value=>createHash('sha256').update(value).digest('hex');
const document=(faces,status='loaded')=>({fonts:Object.assign(new Set(faces),{status})});
const face=(descriptor,status='loaded')=>({descriptor,status});
const required=doc=>nativeFontRequirements(doc,font=>font.descriptor,hash);
test('MP-08/MP-10/MP-11 only loaded native faces are requirements; duplicate descriptors stay bounded',()=>{
 assert.deepEqual(required(document([face('used'),face('unused','unloaded'),face('failed','error'),face('used')])),[hash('used')]);
 assert.equal(required(document([face('used')],'loading')),null);
});
test('MP-11 an incomplete oversized native font requirement set never becomes a ready empty certificate',()=>{
 const faces=Array.from({length:129},(_,i)=>face(String(i)));
 assert.equal(required(document(faces)),null);
 assert.equal(required(document(faces.slice(0,128))).length,128);
 assert.equal(required(document(Array.from({length:4097},()=>face('same')))),null);
 assert.equal(required(document([face('x'.repeat(4097))])),null);
});
