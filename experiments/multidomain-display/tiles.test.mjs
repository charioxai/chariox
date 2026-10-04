// MD-DISPLAY-02: reconstruction, edge tiles, unchanged images and size changes.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {dirtyTiles} from './tiles.mjs';
const require=createRequire(`${process.env.MD_TOOLS||'/root/.chariox/dev/browser-resume-20260930/agents/display/tools'}/package.json`);
const {PNG}=require('pngjs');
test('MD-DISPLAY-02 tiles reconstruct RGB changes including partial edges',()=>{
 const a=new PNG({width:130,height:129});a.data.fill(255);const b=PNG.sync.read(PNG.sync.write(a));b.data[(128*130+129)*4]=0;
 const ref=PNG.sync.write(b),old=PNG.sync.write(a),tiles=dirtyTiles(ref,old,PNG);
 assert.equal(tiles.length,1);assert.equal(tiles[0].width,2);assert.equal(tiles[0].height,1);
 for(const t of tiles)PNG.bitblt(PNG.sync.read(t.bytes),a,0,0,t.width,t.height,t.x,t.y);
 assert.deepEqual(a.data,b.data);assert.equal(dirtyTiles(ref,ref,PNG).length,0);
 assert.equal(dirtyTiles(ref,null,PNG).length,4);
 assert.equal(dirtyTiles(ref,PNG.sync.write(new PNG({width:1,height:1})),PNG).length,4);
});
