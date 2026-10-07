// MD-DISPLAY-02/04: sparse helper packets preserve immutable native snapshots.
import test from 'node:test';
import assert from 'node:assert/strict';
import {NativeRaster} from './kernel-browser-native-pipe.mjs';
import {nativeDamageTiles} from './kernel-browser-tiles.mjs';
import {decodePng} from './kernel-browser-pixels.mjs';
test('MD-DISPLAY reconstructs damage without changing queued snapshots',()=>{
 const raster=new NativeRaster(),pixels=Buffer.alloc(8*4*4,1);
 const first=raster.apply({width:8,height:4,length:pixels.length,serial:1},pixels);
 const patch=Buffer.alloc(2*2*4,9);
 const next=raster.apply({width:8,height:4,length:patch.length,serial:2,base_serial:1,patch:[2,1,4,3]},patch);
 assert.equal(next.length,8*4*4);assert.equal(next.pixels[40],9);assert.equal(next.pixels[0],1);assert.equal(first.pixels[40],1);
 assert.throws(()=>raster.apply({width:8,height:4,length:patch.length,serial:3,base_serial:1,patch:[2,1,4,3]},patch),/base/);
});
test('MD-DISPLAY rejects patch before independent base and malformed bounds',()=>{
 for(const patch of [[-1,0,2,2],[0,0,9,2],[0,0,0,2],[0,0,2,1.5]]){
  const raster=new NativeRaster();raster.apply({width:8,height:4,length:128,serial:1},Buffer.alloc(128));
  assert.throws(()=>raster.apply({width:8,height:4,length:16,serial:2,base_serial:1,patch},Buffer.alloc(16)),/patch/);
 }
 assert.throws(()=>new NativeRaster().apply({width:8,height:4,length:16,serial:1,base_serial:0,patch:[0,0,2,2]},Buffer.alloc(16)),/base/);
});

test('MD-DISPLAY-02 sparse snapshots provide exact regions across disjoint patches without flattening earlier frames',()=>{
 const raster=new NativeRaster(),width=128,height=160;
 const initial=Buffer.alloc(width*height*4,1),first=raster.apply({width,height,length:initial.length,serial:1},initial);
 const second=raster.apply({width,height,length:16,serial:2,base_serial:1,patch:[126,63,128,65]},Buffer.alloc(16,9));
 const third=raster.apply({width,height,length:4,serial:3,base_serial:2,patch:[0,159,1,160]},Buffer.alloc(4,7));
 assert.equal(typeof second.readRegion,'function');
 const expected=Buffer.from(initial);
 for(const y of [63,64])expected.fill(9,(y*width+126)*4,(y*width+128)*4);
 assert.deepEqual(second.readRegion(124,62,4,4),Buffer.concat(Array.from({length:4},(_,r)=>expected.subarray(((62+r)*width+124)*4,((62+r)*width+128)*4))));
 assert.deepEqual(first.pixels,initial);assert.deepEqual(second.pixels,expected);
 expected.fill(7,(159*width)*4,(159*width+1)*4);assert.deepEqual(third.pixels,expected);
 assert.deepEqual(second.readRegion(0,159,1,1),Buffer.alloc(4,1),'queued sparse snapshot must retain pre-patch bytes');
 assert.deepEqual(third.readRegion(0,159,1,1),Buffer.alloc(4,7));
 assert.throws(()=>third.readRegion(127,159,2,1),/region/);
});

test('MD-DISPLAY-04 native tiles preserve exact RGB without materializing a full sparse raster',()=>{
 const raster=new NativeRaster(),width=128,height=160;
 const before=Buffer.alloc(width*height*4);for(let n=0;n<before.length;n+=4){before[n]=1;before[n+1]=2;before[n+2]=3}
 raster.apply({width,height,length:before.length,serial:1},before);
 const raw=raster.apply({width,height,length:4,serial:2,base_serial:1,patch:[127,64,128,65],damage:[127,64,128,65]},Buffer.from([7,8,9,0]));raw.format='bgr0';
 const source={width:raw.width,height:raw.height,length:raw.length,damage:raw.damage,format:raw.format,readRegion:raw.readRegion};
 Object.defineProperty(source,'pixels',{get(){assert.fail('exact sparse repair must not flatten the full raster')}});
 assert.equal(nativeDamageTiles(source,true),true);
 const [tile]=nativeDamageTiles(source),decoded=decodePng(tile.data_base64);
 assert.deepEqual([...decoded.pixels.subarray(0,4)],[3,2,1,255]);
 const changed=(127-tile.x)*4;
 assert.deepEqual([...decoded.pixels.subarray(changed,changed+4)],[9,8,7,255]);
});
