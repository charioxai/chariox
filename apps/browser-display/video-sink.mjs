// MP-08/MP-10/MP-11: whole video goes to the compositor; repairs keep an
// canvas overlay until the next complete decoded video image (key or delta).
// No producer or protection-policy changes.
export class VideoSink {
 constructor(canvas,onTiming){
  this.canvas=canvas;this.onTiming=onTiming;this.pending=[];this.closed=false;this.overlayActive=false;
  this.visibility=()=>{if(canvas.ownerDocument?.hidden){if(this.raf!==undefined)this.cancelRaf?.(this.raf);this.commitPending(false);}};
  canvas.ownerDocument?.addEventListener?.('visibilitychange',this.visibility);
 }
 setup(decoder){
  if(this.decoder===decoder)return Boolean(this.track);
  this.freeze();this.stopTrack();this.decoder=decoder;
  const document=this.canvas.ownerDocument,parent=this.canvas.parentElement;
  if(!document?.createElement||!parent)return false;
  let track;
  if(globalThis.MediaStreamTrackGenerator&&globalThis.MediaStream){
   try{track=new MediaStreamTrackGenerator({kind:'video'});decoder.setSink(track.writable);}
   catch{track?.stop();track=null;}
  }
  const video=document.createElement(track?'video':'canvas');
  this.track=track;this.video=video;
  if(track){video.muted=true;video.autoplay=true;video.playsInline=true;video.srcObject=new MediaStream([track]);}
  video.setAttribute?.('aria-hidden','true');
  if(this.canvas.dataset)this.canvas.dataset.displaySink=track?'track':'canvas';
  if(video.dataset)video.dataset.displaySink=track?'track':'canvas';
  Object.assign(video.style,{position:'absolute',pointerEvents:'none'});
  if(document.defaultView?.getComputedStyle(parent).position==='static'){this.parent=parent;this.parentPosition=parent.style.position;parent.style.position='relative';}
  this.canvasOpacity=this.canvas.style.opacity;this.canvasBackground=this.canvas.style.background;
  if(document.defaultView?.getComputedStyle(this.canvas).position==='static'){this.canvasPosition=this.canvas.style.position;this.canvas.style.position='relative';}
  parent.insertBefore(video,this.canvas);
  const sync=()=>{
   video.hidden=this.canvas.hidden;
   Object.assign(video.style,{left:`${this.canvas.offsetLeft??0}px`,top:`${this.canvas.offsetTop??0}px`,width:`${this.canvas.offsetWidth??this.canvas.width}px`,height:`${this.canvas.offsetHeight??this.canvas.height}px`,objectFit:document.defaultView?.getComputedStyle(this.canvas).objectFit??'fill'});
  };
  sync();
  if(globalThis.ResizeObserver){this.resize=new ResizeObserver(sync);this.resize.observe(this.canvas);this.resize.observe(parent);}
  if(globalThis.MutationObserver){this.mutation=new MutationObserver(sync);this.mutation.observe(this.canvas,{attributes:true,attributeFilter:['style','hidden','class']});}
  if(track)video.play().catch(()=>{});
  return Boolean(track);
 }
 restoreStyle(){
  if(this.canvasPosition!==undefined){if(this.canvas.style.position==='relative')this.canvas.style.position=this.canvasPosition;this.canvasPosition=undefined;}
  if(this.canvasOpacity!==undefined)this.canvas.style.opacity=this.canvasOpacity;
  if(this.canvasBackground!==undefined)this.canvas.style.background=this.canvasBackground;
  if(this.parent){this.parent.style.position=this.parentPosition;this.parent=null;}
 }
 stopTrack(){this.resize?.disconnect();this.mutation?.disconnect();this.resize=null;this.mutation=null;this.track?.stop();this.track=null;if(this.video){this.video.pause?.();this.video.srcObject=null;this.video.remove();this.video=null;}this.restoreStyle();}
 // Called only after the ordered video job passed the worker geometry check.
 async commit(result,frame){
  const image=result.image??result;
  if(this.closed)return {drawn:false};
  const decodedAt=result.decodedAt??performance.timeOrigin+performance.now();
  if(result.committedAt!==undefined){
   this.remember(image);
   this.retireOverlay();
   return {...result,drawn:true};
  }
  return new Promise(resolve=>{
   this.pending.push({image,frame,decodedAt,resolve});
   if(this.raf!==undefined)return;
   const view=this.canvas.ownerDocument?.defaultView??globalThis;
   // Hidden documents cannot run rAF. Preserve their latest base immediately:
   // the presenter advances/ACKs the cursor and retires the borrowed images.
   if(this.canvas.ownerDocument?.hidden){this.commitPending(false);return;}
   this.cancelRaf=view.cancelAnimationFrame?.bind(view)??clearTimeout;
   const commit=()=>this.commitPending(!this.canvas.ownerDocument?.hidden);
   this.raf=view.requestAnimationFrame?view.requestAnimationFrame(commit):setTimeout(commit,0);
  });
 }
 commitPending(visible){
  this.raf=undefined;const jobs=this.pending.splice(0),latest=jobs.at(-1);
  if(!latest)return;
  if(!this.closed){
   const {image,frame}=latest,target=this.video??this.canvas;
   if(target.width!==frame.width)target.width=frame.width;
   if(target.height!==frame.height)target.height=frame.height;
   target.getContext('2d').drawImage(image,0,0,frame.width,frame.height);
   this.retireOverlay();
   this.remember(image);
  }
  const committedAt=visible?performance.timeOrigin+performance.now():undefined;
  for(const job of jobs){const drawn=!this.closed&&visible&&job===latest;job.resolve({drawn,decodedAt:job.decodedAt,committedAt});if(!drawn&&!this.closed)this.onTiming('client_present_drop',job.decodedAt);}
 }
 remember(image){this.last?.close();this.last=image.clone?.();}
 retireOverlay(){
  if(!this.video&&!this.track)return;
  // A codec delta reconstructs a complete image. Earlier exact repairs and
  // scroll moves belong to the old image, even when no new key is needed.
  // Clear the bitmap too: snapshots and later patches must not revive it.
  this.canvas.getContext('2d').clearRect(0,0,this.canvas.width,this.canvas.height);
  this.canvas.style.background='transparent';this.canvas.style.opacity='0';this.overlayActive=false;
 }
 overlay(frame){
  if(!this.video&&!this.track)return;
  if(this.canvas.width!==frame.width)this.canvas.width=frame.width;
  if(this.canvas.height!==frame.height)this.canvas.height=frame.height;
  if(!this.overlayActive){
   this.canvas.getContext('2d').clearRect(0,0,frame.width,frame.height);
   this.canvas.style.background='transparent';
   this.canvas.style.opacity=this.canvasOpacity??'';this.overlayActive=true;
  }
 }
 snapshot(context){
  if((this.video||this.track)&&this.last){context.globalCompositeOperation='copy';context.drawImage(this.last,0,0,this.canvas.width,this.canvas.height);context.globalCompositeOperation='source-over';}
  if((!this.video&&!this.track)||this.overlayActive)context.drawImage(this.canvas,0,0);
 }
 freeze(){
  if((this.video||this.track)&&this.last){
   const copy=new OffscreenCanvas(this.canvas.width,this.canvas.height);this.snapshot(copy.getContext('2d'));
   this.canvas.getContext('2d').drawImage(copy,0,0);copy.width=1;copy.height=1;
  }
 }
 close({preserveFrame=false}={}){
  if(preserveFrame)this.freeze();
  this.closed=true;this.canvas.ownerDocument?.removeEventListener?.('visibilitychange',this.visibility);if(this.raf!==undefined)this.cancelRaf?.(this.raf);this.raf=undefined;commitHidden(this.pending);
  this.last?.close();this.last=null;this.stopTrack();
 }
}
function commitHidden(jobs){for(const job of jobs.splice(0)){job.resolve({drawn:false,decodedAt:job.decodedAt});}}
