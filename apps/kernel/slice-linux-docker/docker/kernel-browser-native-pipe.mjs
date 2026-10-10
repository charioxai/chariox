// MD-DISPLAY-02/04: one allocation/copy per raw frame, never quadratic concat.
import {nativeExactReplyLimit} from './kernel-browser-native-worker.mjs';
export class NativePipe {
 constructor(validate,frame){this.validate=validate;this.frame=frame;this.buffer=Buffer.alloc(4);this.offset=0;this.stage='length';}
 push(bytes){
  let at=0;
  while(at<bytes.length){
   const n=Math.min(bytes.length-at,this.buffer.length-this.offset);bytes.copy(this.buffer,this.offset,at,at+n);this.offset+=n;at+=n;
   if(this.offset!==this.buffer.length)continue;
   if(this.stage==='length'){
    const size=this.buffer.readUInt32BE(0);if(size<1||size>8192)throw Error('MD-DISPLAY: native header bound');this.buffer=Buffer.alloc(size);this.stage='header';
   }else if(this.stage==='header'){
    const header=JSON.parse(this.buffer);this.validate(header);
    const limit=header.reply===undefined?2560*1600*4:nativeExactReplyLimit;
    if(!Number.isSafeInteger(header.length)||header.length<1||header.length>limit)throw Error('MD-DISPLAY: native frame bound');
    this.header=header;this.buffer=Buffer.allocUnsafe(header.length);this.stage='pixels';
   }else{
    this.frame(this.header,this.buffer);this.header=null;this.buffer=Buffer.alloc(4);this.stage='length';
   }
   this.offset=0;
  }
 }
}

// Private helper IPC only. Every sparse packet follows its immediately preceding
// raster; retain an immutable complete snapshot for capture/encode concurrency.
class RasterBands {
 constructor(width,height,pixels,previous=null,patch=null){
  this.width=width;this.height=height;this.stride=width*4;this.bandBytes=this.stride*64;
  if(!patch){
   this.flat=pixels;this.bands=[];
   for(let start=0;start<pixels.length;start+=this.bandBytes)this.bands.push(pixels.subarray(start,Math.min(pixels.length,start+this.bandBytes)));
  }else{
   this.bands=previous.bands.slice();const [left,top,right,bottom]=patch;
   for(let band=Math.floor(top/64);band<=Math.floor((bottom-1)/64);band++)this.bands[band]=Buffer.from(this.bands[band]);
   for(let row=top;row<bottom;row++)pixels.copy(this.bands[Math.floor(row/64)],((row%64)*width+left)*4,(row-top)*(right-left)*4,(row-top+1)*(right-left)*4);
  }
 }
 pixels(){return this.flat??=Buffer.concat(this.bands,this.width*this.height*4)}
 region(x,y,width,height){
  if(![x,y,width,height].every(Number.isSafeInteger)||x<0||y<0||width<1||height<1||x+width>this.width||y+height>this.height)throw Error('MD-DISPLAY: raster region bounds');
  const result=Buffer.allocUnsafe(width*height*4);
  for(let row=0;row<height;row++)this.bands[Math.floor((y+row)/64)].copy(result,row*width*4,(((y+row)%64)*this.width+x)*4,(((y+row)%64)*this.width+x+width)*4);
  return result;
 }
}
export class NativeRaster {
 apply(header,pixels){
  const {width,height,serial,length,patch}=header;
  if(![width,height,serial,length].every(Number.isSafeInteger)||width<1||width>2560||height<1||height>1600||serial<1||length!==pixels.length)throw Error('MD-DISPLAY: raster bound');
  if(patch){
   if(!Array.isArray(patch)||patch.length!==4||!patch.every(Number.isSafeInteger))throw Error('MD-DISPLAY: patch shape');
   const [left,top,right,bottom]=patch;
   if(left<0||top<0||right<=left||bottom<=top||right>width||bottom>height||length!==(right-left)*(bottom-top)*4)throw Error('MD-DISPLAY: patch bounds');
   if(!this.previous||this.previous.width!==width||this.previous.height!==height||this.previous.serial!==header.base_serial||serial!==header.base_serial+1)throw Error('MD-DISPLAY: raster base lost');
  }else if(length!==width*height*4)throw Error('MD-DISPLAY: full raster bounds');
  // Copy only damaged 64-row bands; unchanged bands are shared immutably.
  // Exact tiles read their small regions directly. Motion materializes one
  // contiguous raster lazily at encode time, outside the capture/input path.
  this.bands=new RasterBands(width,height,pixels,this.bands,patch);
  const bands=this.bands;
  this.previous={...header,length:width*height*4};
  Object.defineProperties(this.previous,{
   pixels:{enumerable:true,get:()=>bands.pixels()},
   readRegion:{value:(...args)=>bands.region(...args)},
  });
  return this.previous;
 }
}
