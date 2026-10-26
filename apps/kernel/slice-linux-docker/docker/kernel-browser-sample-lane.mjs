// MD-DISPLAY-02/04: CDP screenshot may temporarily change viewport geometry.
// Serialize physical input with sampling only, never with decode/encode/pacing.
export class SampleLane {
  constructor() { this.active=false;this.waiting=[]; }
  run(kind,operation) {
    if(this.waiting.length>=64)return Promise.reject(Error('MD-DISPLAY: sample lane full'));
    return new Promise((resolve,reject)=>{this.waiting.push({kind,operation,resolve,reject});this.pump()});
  }
  pump() {
    if(this.active||!this.waiting.length)return;
    const input=this.waiting.findIndex(x=>x.kind==='input');
    const next=this.waiting.splice(input<0?0:input,1)[0];this.active=true;
    Promise.resolve().then(next.operation).then(next.resolve,next.reject).finally(()=>{this.active=false;this.pump()});
  }
}
