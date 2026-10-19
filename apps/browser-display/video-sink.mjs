// MP-08/MP-10/MP-11: whole video goes to the compositor; repairs keep an
// canvas overlay until the next complete decoded video image (key or delta).
// No producer or protection-policy changes.
const SUSTAINED_FRAMES=8,SUSTAINED_MS=250,SUSTAINED_WAITING=3;
export class VideoSink {
 constructor(canvas,onTiming,{direct=false,paced=false}={}){
  this.canvas=canvas;this.onTiming=onTiming;this.direct=direct;this.paced=paced&&direct;this.frameDrawn=false;this.pending=[];this.closed=false;this.overlayActive=false;this.presentationJobs=new Map();this.presentationMarks=new Map();this.lastPresentedTimestamp=-Infinity;
  this.visibility=()=>{if(canvas.ownerDocument?.hidden){if(this.raf!==undefined)this.cancelRaf?.(this.raf);this.endFrame();this.commitPending(false);}};
  canvas.ownerDocument?.addEventListener?.('visibilitychange',this.visibility);
 }
 setup(decoder){
  if(this.decoder===decoder)return Boolean(this.track);
  this.freeze();this.stopTrack();this.decoder=decoder;
  const document=this.canvas.ownerDocument,parent=this.canvas.parentElement;
  if(!document?.createElement||!parent)return false;
  let track;
  if(!this.direct&&globalThis.MediaStreamTrackGenerator&&globalThis.MediaStream){
   try{track=new MediaStreamTrackGenerator({kind:'video'});decoder.setSink(track.writable);}
   catch{track?.stop();track=null;}
  }
  const video=document.createElement(track?'video':'canvas');
  // MP-08/MP-10/MP-11: video pixels use this sibling, not the repair overlay.
  if(this.direct&&!track)video.getContext?.('2d',{desynchronized:true});
  this.track=track;this.video=video;
  if(this.canvas.dataset){delete this.canvas.dataset.displayPresentedSequence;delete this.canvas.dataset.displayCommittedAt;}
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
  if(track){this.observePresentation(video);video.play().catch(()=>{});}
  return Boolean(track);
 }
 restoreStyle(){
  if(this.canvasPosition!==undefined){if(this.canvas.style.position==='relative')this.canvas.style.position=this.canvasPosition;this.canvasPosition=undefined;}
  if(this.canvasOpacity!==undefined)this.canvas.style.opacity=this.canvasOpacity;
  if(this.canvasBackground!==undefined)this.canvas.style.background=this.canvasBackground;
  if(this.parent){this.parent.style.position=this.parentPosition;this.parent=null;}
 }
 // MP-08/MP-10/MP-11: writer completion only queues pixels. Join rVFC's
 // exact media timestamp to the decoded image, including callbacks arriving
 // before the worker reply. These bounded maps hold numeric metadata only.
 observePresentation(video){
  if(this.closed||this.video!==video||!video.requestVideoFrameCallback)return;
  this.videoCallback=video.requestVideoFrameCallback((_now,metadata)=>{
   if(this.closed||this.video!==video)return;
   const timestamp=Math.round(metadata.mediaTime*1000000);
   if(Number.isSafeInteger(timestamp)&&Number.isFinite(metadata.presentationTime)){
    const mark={committedAt:performance.timeOrigin+metadata.presentationTime,expectedDisplayAt:performance.timeOrigin+metadata.expectedDisplayTime,presentedFrames:metadata.presentedFrames};
    this.presentationMarks.set(timestamp,mark);
    this.publishPresentation(timestamp);
    while(this.presentationMarks.size>32)this.presentationMarks.delete(this.presentationMarks.keys().next().value);
   }
   this.observePresentation(video);
  });
 }
 publishPresentation(timestamp){
  const job=this.presentationJobs.get(timestamp),mark=this.presentationMarks.get(timestamp);
  if(!job||!mark)return;
  this.presentationJobs.delete(timestamp);this.presentationMarks.delete(timestamp);
  Object.assign(job.outcome,mark);
  if(timestamp>this.lastPresentedTimestamp&&this.canvas.dataset){this.lastPresentedTimestamp=timestamp;this.canvas.dataset.displayPresentedSequence=String(job.sequence);this.canvas.dataset.displayCommittedAt=String(mark.committedAt);}
  this.onTiming('client_present',job.outcome.writtenAt,{sequence:job.sequence,...mark,decodedAt:job.outcome.decodedAt,writtenAt:job.outcome.writtenAt,mediaTimestamp:timestamp});
  for(const [older,pending]of this.presentationJobs)if(older<timestamp){this.presentationJobs.delete(older);this.onTiming('client_present_drop',pending.outcome.decodedAt,{sequence:pending.sequence});}
 }
 stopTrack(){if(this.videoCallback!==undefined)this.video?.cancelVideoFrameCallback?.(this.videoCallback);this.videoCallback=undefined;this.presentationJobs.clear();this.presentationMarks.clear();this.lastPresentedTimestamp=-Infinity;this.resize?.disconnect();this.mutation?.disconnect();this.resize=null;this.mutation=null;this.track?.stop();this.track=null;if(this.video){this.video.pause?.();this.video.srcObject=null;this.video.remove();this.video=null;}this.restoreStyle();}
 // Called only after the ordered video job passed the worker geometry check.
 async commit(result,frame){
  const image=result.image??result;
  if(this.closed)return {drawn:false};
  const decodedAt=result.decodedAt??performance.timeOrigin+performance.now();
  if(result.writtenAt!==undefined){
   this.remember(image);
   this.retireOverlay();
   const outcome={decodedAt,writtenAt:result.writtenAt,drawn:true};
   if(Number.isSafeInteger(image.timestamp)&&this.video?.requestVideoFrameCallback){
    this.presentationJobs.set(image.timestamp,{sequence:frame.sequence,outcome});
    this.publishPresentation(image.timestamp);
    while(this.presentationJobs.size>32)this.presentationJobs.delete(this.presentationJobs.keys().next().value);
   }
   return outcome;
  }
  return new Promise(resolve=>{
   this.pending.push({image,frame,decodedAt,resolve});
   if(this.raf!==undefined)return;
   const view=this.canvas.ownerDocument?.defaultView??globalThis;
   // Hidden documents cannot run rAF. Preserve their latest base immediately:
   // the presenter advances/ACKs the cursor and retires the borrowed images.
   if(this.canvas.ownerDocument?.hidden){this.commitPending(false);return;}
   // MP-08/MP-10/MP-11: an intermediate region canvas is copied into a
   // visible surface by onPresented; only that surface needs a paint boundary.
   if(this.paced&&this.direct){this.commitPaced(view);return;}
   if(this.direct||this.canvas.isConnected===false){this.commitPending(true);return;}
   this.cancelRaf=view.cancelAnimationFrame?.bind(view)??clearTimeout;
   const commit=()=>this.commitPending(!this.canvas.ownerDocument?.hidden);
   this.raf=view.requestAnimationFrame?view.requestAnimationFrame(commit):setTimeout(commit,0);
  });
 }
 // vidlat r39: a page crop shows one decoded frame per compositor frame. Frames that arrive within
 // one frame used to overwrite each other (~15 % of the draws never reached the screen); each later
 // one now waits for the next frame. Decode-ahead hands over several at once: only the newest of them waits.
 commitPaced(view){
  // Sparse updates (typing) keep one waiting frame, so a burst never queues the page up behind the compositor. Sustained motion
  // (video, a long scroll) delivers near the display rate and its jitter must not cost frames: up to three may wait.
  const waiting=this.sustained()?SUSTAINED_WAITING:1;
  while(this.frameDrawn&&this.pending.length>waiting){const job=this.pending.shift();job.resolve({drawn:false,decodedAt:job.decodedAt});this.onTiming('client_present_drop',job.decodedAt);}
  if(!this.frameDrawn){this.frameDrawn=true;this.armFrame(view);this.commitOldest();return;}
  this.armFrame(view);
 }
 // At least SUSTAINED_FRAMES commits within SUSTAINED_MS: the page is in continuous motion.
 sustained(){
  const now=performance.now(),recent=this.recent??=[];
  recent.push(now);if(recent.length>SUSTAINED_FRAMES)recent.shift();
  return recent.length===SUSTAINED_FRAMES&&now-recent[0]<SUSTAINED_MS;
 }
 armFrame(view){
  if(this.frameRaf!==undefined||!view.requestAnimationFrame)return;
  this.cancelFrame=view.cancelAnimationFrame?.bind(view);
  // A document whose animation frames do not run (throttled or detached) must not hold decoded frames: a timer ends the frame too.
  let ended=false,timer;
  const end=()=>{
   if(ended)return;ended=true;clearTimeout(timer);
   this.frameRaf=undefined;this.frameDrawn=false;
   if(this.closed||!this.pending.length)return;
   // The waiting frame is drawn after this frame's commit, so the next compositor frame shows it.
   this.frameDrawn=true;this.armFrame(view);
   setTimeout(()=>{if(!this.closed)this.commitOldest();},0);
  };
  this.frameRaf=view.requestAnimationFrame(end);
  timer=setTimeout(()=>{if(!ended){this.cancelFrame?.(this.frameRaf);end();}},50);
  this.frameTimer=()=>{ended=true;clearTimeout(timer);};
 }
 endFrame(){if(this.frameRaf!==undefined)this.cancelFrame?.(this.frameRaf);this.frameTimer?.();this.frameRaf=undefined;this.frameDrawn=false;}
 commitOldest(){const job=this.pending.shift();if(!job)return;const rest=this.pending;this.pending=[job];this.commitPending(true);this.pending=rest;}
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
  this.closed=true;this.canvas.ownerDocument?.removeEventListener?.('visibilitychange',this.visibility);if(this.raf!==undefined)this.cancelRaf?.(this.raf);this.raf=undefined;this.endFrame();commitHidden(this.pending);
  this.last?.close();this.last=null;this.stopTrack();
 }
}
function commitHidden(jobs){for(const job of jobs.splice(0)){job.resolve({drawn:false,decodedAt:job.decodedAt});}}
