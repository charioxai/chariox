// MD-DISPLAY-02: trusted browser-native RGB differencing and atomic PNG batches.
// Research only: production needs epochs, masks, resync and actual network pacing.
class ExactFrameEncoder {
  constructor({pack,emit,budget}){this.pack=pack;this.emit=emit;this.budget=budget;this.reset()}
  reset(){this.previous=null;this.credits=new Map();this.stats={frames:0,full_frames:0,tiles:0,bytes:0,initial_bytes:0}}
  credit(id){this.credits.get(id)?.();this.credits.delete(id)}
  async encode(meta,bytes){
    const began=performance.now(),bmp=await createImageBitmap(new Blob([bytes],{type:'image/png'}));
    if(!this.canvas||this.canvas.width!==bmp.width||this.canvas.height!==bmp.height){this.canvas=new OffscreenCanvas(bmp.width,bmp.height);this.ctx=this.canvas.getContext('2d',{willReadFrequently:true,alpha:false});this.previous=null}
    this.ctx.drawImage(bmp,0,0);bmp.close();const image=this.ctx.getImageData(0,0,this.canvas.width,this.canvas.height),pixels=new Uint32Array(image.data.buffer);
    const tiles=[],size=128,w=this.canvas.width,h=this.canvas.height;
    if(this.previous)for(let y=0;y<h;y+=size)for(let x=0;x<w;x+=size){
      const width=Math.min(size,w-x),height=Math.min(size,h-y);let changed=false;
      for(let j=y;!changed&&j<y+height;j++)for(let i=x;!changed&&i<x+width;i++)changed=pixels[j*w+i]!==this.previous[j*w+i];
      if(changed)tiles.push({x,y,width,height});
    }
    if(this.previous&&!tiles.length){this.emit(JSON.stringify({kind:'exact-noop',timestamp:meta.timestamp}));return}
    let full=!this.previous,packets=[];
    if(!full){
      // Bound native PNG work to eight tiles at a time.
      for(let at=0;at<tiles.length;at+=8)packets.push(...await Promise.all(tiles.slice(at,at+8).map(async(r,i)=>{
        const c=new OffscreenCanvas(r.width,r.height);c.getContext('2d',{alpha:false}).drawImage(this.canvas,r.x,r.y,r.width,r.height,0,0,r.width,r.height);
        const png=new Uint8Array(await(await c.convertToBlob({type:'image/png'})).arrayBuffer());
        return this.pack({kind:'exact-tile',timestamp:meta.timestamp,total:tiles.length,id:`${meta.timestamp}-${at+i}`,...r},png);
      })));
      full=packets.reduce((n,p)=>n+p.length,0)>bytes.length;
    }
    if(full){packets=[this.pack({kind:'exact',timestamp:meta.timestamp},bytes)];this.stats.full_frames++;if(!this.previous)this.stats.initial_bytes=packets[0].length}
    else this.stats.tiles+=tiles.length;
    let sent=0;const credits=[];
    for(let i=0;i<packets.length;i++){
      if(!full)credits.push(new Promise(r=>this.credits.set(`${meta.timestamp}-${i}`,r)));
      sent+=packets[i].length;await new Promise(r=>setTimeout(r,Math.max(0,began+sent*8000/this.budget-performance.now())));
      this.emit(packets[i]);if(!full&&i>=7)await credits[i-7];
    }
    await Promise.all(credits);this.previous=pixels;this.stats.frames++;this.stats.bytes+=sent;
  }
}
class ExactFrameDecoder {
  constructor({ctx,painted,credit}){Object.assign(this,{ctx,painted,credit});this.batches=new Map()}
  reset(){for(const b of this.batches.values())for(const t of b.tiles)t.bmp.close();this.batches.clear()}
  async accept(meta,bytes){
    const bmp=await createImageBitmap(new Blob([bytes],{type:'image/png'}));
    if(meta.kind==='exact'){this.reset();this.ctx.drawImage(bmp,0,0);bmp.close();await this.painted('exact',meta.timestamp);return}
    if(!Number.isInteger(meta.total)||meta.total<1||meta.total>256){bmp.close();throw Error('MD-DISPLAY exact batch bound')}
    if(!this.batches.has(meta.timestamp))this.batches.set(meta.timestamp,{total:meta.total,tiles:[]});
    const b=this.batches.get(meta.timestamp);b.tiles.push({...meta,bmp});this.credit(meta.id);
    if(b.tiles.length===b.total){for(const t of b.tiles){this.ctx.drawImage(t.bmp,t.x,t.y);t.bmp.close()}this.batches.delete(meta.timestamp);await this.painted('exact',meta.timestamp)}
  }
}
window.ExactFrameEncoder=ExactFrameEncoder;window.ExactFrameDecoder=ExactFrameDecoder;
