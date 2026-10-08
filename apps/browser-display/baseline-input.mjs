// MP-10: matches upstream gst-web/src/input.js positive DOM wheel mapping.
export function wheelMessages(x,y,deltaY){
  if(![x,y,deltaY].every(Number.isSafeInteger)||x<0||x>=1920||y<0||y>=1080||!deltaY||Math.abs(deltaY)>7680)throw Error('MP-10 invalid baseline wheel');
  const mask=deltaY>0?8:16,magnitude=Math.max(1,Math.round(Math.abs(deltaY)/120));
  return [`m,${x},${y},${mask},${magnitude}`,`m,${x},${y},0,0`];
}
