// MP-10: identical content-change sampling; repeated video frames are separate.
export function changedPixels(before, after) {
  if (!before) return true;
  let changed=0;
  for(let i=0;i<after.length;i+=4)
    if(Math.max(...[0,1,2].map(c=>Math.abs(after[i+c]-before[i+c])))>8)changed++;
  return changed>after.length/4*.001;
}
export class MotionSamples {
  constructor(){this.canvas=new OffscreenCanvas(64,36);this.context=this.canvas.getContext('2d',{willReadFrequently:true});this.previous=null;}
  sample(canvas){
    // Exclude the fixed input/probe header. This is a declared thumbnail
    // threshold, not proof of exact raster identity or source frame alignment.
    this.context.drawImage(canvas,0,60,canvas.width,canvas.height-60,0,0,64,36);
    const pixels=this.context.getImageData(0,0,64,36).data;
    const changed=changedPixels(this.previous,pixels);this.previous=pixels;return changed;
  }
}
