// MD-DISPLAY-04: bounded decoded exact tiles, scoped to one admitted document.
export class TileCache {
 constructor(limit=256){this.limit=limit;this.closed=false;this.values=new Map();this.bytes=0;this.pinned=new Set();this.owned=new WeakSet();}
 begin(){this.pinned.clear()}
 end(){this.pinned.clear()}
 async get(data,width,height,decode){
  if(this.closed)throw Error('MD-DISPLAY: tile cache closed');
  const found=this.values.get(data);
  if(found){this.values.delete(data);this.values.set(data,found);this.pinned.add(data);return found.bitmap}
  const bitmap=await decode();if(this.closed){bitmap.close();throw Error('MD-DISPLAY: tile cache closed')}if(bitmap.width!==width||bitmap.height!==height){bitmap.close();throw Error('MD-DISPLAY: tile dimensions')}
  const cost=width*height*4+data.length*2;
  while(this.values.size&& (this.values.size>=this.limit||this.bytes+cost>24*1024*1024)){const key=[...this.values.keys()].find(key=>!this.pinned.has(key));if(key===undefined)return bitmap;const old=this.values.get(key);this.values.delete(key);this.bytes-=old.cost;this.owned.delete(old.bitmap);old.bitmap.close()}
  this.values.set(data,{bitmap,cost});this.bytes+=cost;this.pinned.add(data);this.owned.add(bitmap);return bitmap;
 }
 close(){this.closed=true;this.clear()}
 clear(){for(const {bitmap}of this.values.values()){this.owned.delete(bitmap);bitmap.close()};this.values.clear();this.bytes=0}
}
