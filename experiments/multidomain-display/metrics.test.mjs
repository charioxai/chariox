// MD-DISPLAY-02: protect measurements against false lossless/percentile claims.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import path from 'node:path';
import { compare, distribution } from './metrics.mjs';
const require=createRequire(path.resolve(process.env.MD_TOOLS||'/root/.chariox/dev/browser-resume-20260930/agents/display/tools','package.json'));
const {PNG}=require('pngjs');
function png(width,rgb){const image=new PNG({width,height:1});for(let i=0;i<width;i++){image.data.set([...rgb,255],i*4)}return PNG.sync.write(image)}
test('MD-DISPLAY-02 fidelity rejects different dimensions',()=>{const r=compare(png(1,[0,0,0]),png(2,[0,0,0]),PNG);assert.equal(r.comparable,false);assert.equal(r.lossless,undefined)});
test('MD-DISPLAY-02 exact pixels and known RGB error have distinct results',()=>{const black=png(1,[0,0,0]),r=compare(black,png(1,[255,0,0]),PNG);assert.equal(compare(black,black,PNG).lossless,true);assert.equal(r.lossless,false);assert.equal(r.mse_rgb,255**2/3);assert.ok(Math.abs(r.psnr_db-10*Math.log10(3))<1e-10);assert.equal(r.changed_pixels_fraction,1)});
test('MD-DISPLAY-02 latency retains tails and empty distributions',()=>{assert.equal(distribution([]).p95_ms,null);const r=distribution([1,2,3,4,900]);assert.equal(r.p50_ms,3);assert.equal(r.p95_ms,900);assert.equal(r.max_ms,900);assert.equal(r.histogram.reduce((sum,b)=>sum+b.count,0),5)});
