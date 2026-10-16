// MD-DISPLAY-04: optional trusted-rectangle preview; never an authoritative ack.
// Unsupported/fixed/sticky page geometry keeps this disabled. All authoritative
// frames and new input restore the actual canvas before using any delta base.
export class ScrollPrediction {
 constructor(canvas,rect,snapshot=context=>context.drawImage(canvas,0,0)){this.canvas=canvas;this.rect=rect;this.snapshot=snapshot;this.saved=null;}
 restore(){
  if(!this.saved)return;
  if(this.preview){this.preview.remove();this.preview.width=1;this.preview.height=1;this.preview=null;}
  else{const context=this.canvas.getContext('2d');context.save();context.globalCompositeOperation='copy';context.drawImage(this.saved,0,0);context.restore();}
  if(this.parent){this.parent.style.position=this.parentPosition;this.parent=null;}
  this.saved.width=1;this.saved.height=1;this.saved=null;
 }
 predict(input,scale){
  this.restore();const r=this.rect;
  if(input.kind!=='scroll'||input.delta_x||!r||![r.x,r.y,r.width,r.height].every(Number.isSafeInteger)||r.x<0||r.y<0||r.width<1||r.height<1||r.x+r.width>1280||r.y+r.height>800)return false;
  const shift=Math.round(input.delta_y*scale);if(!shift||Math.abs(shift)>=r.height*scale)return false;
  this.saved=new OffscreenCanvas(this.canvas.width,this.canvas.height);this.snapshot(this.saved.getContext('2d'));
  const document=this.canvas.ownerDocument,parent=this.canvas.parentElement;
  if(document?.createElement&&parent){
   const preview=document.createElement('canvas');this.preview=preview;
   preview.width=this.canvas.width;preview.height=this.canvas.height;preview.setAttribute('aria-hidden','true');
   const style=document.defaultView.getComputedStyle(this.canvas);
   Object.assign(preview.style,{position:'absolute',pointerEvents:'none',left:`${this.canvas.offsetLeft}px`,top:`${this.canvas.offsetTop}px`,width:`${this.canvas.offsetWidth}px`,height:`${this.canvas.offsetHeight}px`,objectFit:style.objectFit});
   if(document.defaultView.getComputedStyle(parent).position==='static'){this.parent=parent;this.parentPosition=parent.style.position;parent.style.position='relative';}
   parent.insertBefore(preview,this.canvas.nextSibling);
   preview.getContext('2d').drawImage(this.saved,0,0);
  }
  const x=r.x*scale,y=r.y*scale,w=r.width*scale,h=r.height*scale,dy=Math.max(0,shift),sy=Math.max(0,-shift),height=h-Math.abs(shift);
  const context=(this.preview??this.canvas).getContext('2d');
  // Copy just the trusted rectangle. Transparent source pixels must also erase
  // the destination; source-over would leave shifted repairs behind.
  context.save();context.beginPath();context.rect(x,y+sy,w,height);context.clip();context.globalCompositeOperation='copy';
  context.drawImage(this.saved,x,y+dy,w,height,x,y+sy,w,height);context.restore();return true;
 }
 close(){this.restore()}
}
