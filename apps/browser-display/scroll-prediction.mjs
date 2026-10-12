// MD-DISPLAY-04: optional trusted-rectangle preview; never an authoritative ack.
// Unsupported/fixed/sticky page geometry keeps this disabled. All authoritative
// frames and new input restore the actual canvas before using any delta base.
export class ScrollPrediction {
 constructor(canvas,rect){this.canvas=canvas;this.rect=rect;this.saved=null;}
 restore(){if(!this.saved)return;this.canvas.getContext('2d').drawImage(this.saved,0,0);this.saved.width=1;this.saved.height=1;this.saved=null;}
 predict(input,scale){
  this.restore();const r=this.rect;
  if(input.kind!=='scroll'||input.delta_x||!r||![r.x,r.y,r.width,r.height].every(Number.isSafeInteger)||r.x<0||r.y<0||r.width<1||r.height<1||r.x+r.width>1280||r.y+r.height>800)return false;
  const shift=Math.round(input.delta_y*scale);if(!shift||Math.abs(shift)>=r.height*scale)return false;
  this.saved=new OffscreenCanvas(this.canvas.width,this.canvas.height);this.saved.getContext('2d').drawImage(this.canvas,0,0);
  const x=r.x*scale,y=r.y*scale,w=r.width*scale,h=r.height*scale,dy=Math.max(0,shift),sy=Math.max(0,-shift),height=h-Math.abs(shift);
  this.canvas.getContext('2d').drawImage(this.saved,x,y+dy,w,height,x,y+sy,w,height);return true;
 }
 close(){this.restore()}
}
