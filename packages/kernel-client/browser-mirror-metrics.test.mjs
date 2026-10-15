// MP-10: the fidelity gate must expose holes and color changes, not hide them.
import test from 'node:test';import assert from 'node:assert/strict';
import {mirrorRasterMetrics} from './browser-mirror-metrics.mjs';
const raster=()=>({width:20,height:20,data:Buffer.alloc(20*20*4,255)});
test('MP-10: exact pairs have zero residual and uniform color drift remains visible',()=>{const a=raster(),b=raster();assert.equal(mirrorRasterMetrics(a,b).outside_edge_mismatch_pixels,0);b.data[4*200]=0;assert.equal(mirrorRasterMetrics(a,b).outside_edge_mismatch_pixels,1);});
test('MP-10: client-created holes cannot create exclusions',()=>{const a=raster(),b=raster();b.data.fill(0,4*100,4*120);const m=mirrorRasterMetrics(a,b);assert.equal(m.edge_band_pixels,0);assert.equal(m.outside_edge_mismatch_pixels,20);});
