// MP-08/MP-10/MP-11: numeric observation only, never changes viewer pixels.
export function sampleVisibleFrame(canvas,frames,inputAt,dark){
 const frame=frames.at(-1);
 if(!frame||frame.drawn<=inputAt)return null;
 const base=canvas.previousElementSibling;
 let context=canvas.getContext('2d');
 if(base?.dataset?.displaySink){
  const video=base.tagName==='VIDEO';
  if(base.hidden||video&&(base.readyState<2||!base.videoWidth||!base.videoHeight))return null;
  const sample=canvas.ownerDocument.createElement('canvas');sample.width=10;sample.height=10;
  context=sample.getContext('2d');
  const draw=(image,width,height)=>context.drawImage(image,2*width/1280,100*height/800,10*width/canvas.width,10*height/canvas.height,0,0,10,10);
  draw(base,video?base.videoWidth:base.width,video?base.videoHeight:base.height);
  if(canvas.style.opacity!=='0')draw(canvas,canvas.width,canvas.height);
 }
 const data=base?.dataset?.displaySink?context.getImageData(0,0,10,10).data:context.getImageData(2*canvas.width/1280,100*canvas.height/800,10,10).data;
 let sum=0;for(let i=0;i<data.length;i+=4)sum+=(data[i]+data[i+1]+data[i+2])/3;
 const mean=sum/(data.length/4);
 return frame?.drawn>inputAt&&(dark?mean<100:mean>180)?{frame,mean}:null;
}
