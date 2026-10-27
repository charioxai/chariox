// MP-08/MP-10/MP-11: decode at most three contiguous whole-video frames
// ahead of the ordered commit cursor. Repairs are barriers, never bypassed.
export class PresentationQueue {
 constructor(presenter,canPrepare=()=>true){this.presenter=presenter;this.canPrepare=canPrepare;this.jobs=[];this.prepared=new Set();this.tail=Promise.resolve();}
 submit(frame,consume){
  const job={frame};this.jobs.push(job);this.prepare();
  const result=this.tail.then(()=>consume(frame));
  this.tail=result.catch(()=>{}).finally(()=>{this.jobs.shift();this.prepared.delete(job);this.prepare();});
  return result;
 }
 cancel(){this.closed=true;for(const {frame}of this.jobs)this.presenter.discardPrepared?.(frame);}
 prepare(){
  if(this.closed)return;
  const first=this.jobs[0]?.frame;
  for(const job of this.jobs){
   if(!this.canPrepare(job.frame)||job.frame.kind!=='video'||job.frame.document_id!==first.document_id||job.frame.width!==first.width||job.frame.height!==first.height||job.frame.codec!==first.codec)break;
   if(this.prepared.has(job))continue;
   if(this.prepared.size>=3)break;
   this.prepared.add(job);this.presenter.prepareVideo?.(job.frame);
  }
 }
}
