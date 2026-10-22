// MP-08/MP-10/MP-11: one shared policy for ordinary and hosted catalog reads.
// Scope belongs to a transport/authority generation. No persisted cache, no
// mutation sharing, and no catalog or credential values enter diagnostics.
export function isKernelCatalogRead(request:unknown):boolean {
 return request!==null&&typeof request==='object'&&!Array.isArray(request)&&Object.keys(request).length===1&&Object.hasOwn(request,'GetProviderCatalog')
}
// Capture before protocol/admission checks and asynchronous display waits.
export function captureKernelCatalogRead(request:unknown):unknown { return isKernelCatalogRead(request)?JSON.parse(JSON.stringify(request)):request }
export class KernelCatalogReads {
 private readonly pending=new Map<string,Promise<unknown>>()
 private readonly holds=new Set<()=>void>()
 private readonly waiting=new Set<()=>void>()
 private epoch=0
 // MP-08/MP-10: defer background catalogs through the initial display paint,
 // bounded to five seconds per waiter. Mutation and explicit-admission paths stay live.
 hold():()=>void{
  let timer:ReturnType<typeof setTimeout>
  const release=()=>{if(!this.holds.delete(release))return;clearTimeout(timer);if(!this.holds.size)for(const finish of [...this.waiting])finish()}
  this.holds.add(release);timer=setTimeout(release,5000);return release
 }
 private displayReady():Promise<void>{
  return new Promise(resolve=>{
   const finish=()=>{clearTimeout(timer);this.waiting.delete(finish);resolve()}
   const timer=setTimeout(finish,5000);this.waiting.add(finish)
  })
 }
 run<T>(request:unknown,scope:string,send:()=>Promise<T>):Promise<T>{
  if(!isKernelCatalogRead(request))return send()
  const key=JSON.stringify([scope,request]),existing=this.pending.get(key)
  if(existing)return existing as Promise<T>
  if(this.pending.size>=32)return send()
  const epoch=this.epoch
  const start=()=>{if(epoch!==this.epoch)throw Error("Kernel catalog read cancelled");return send()}
  const run=this.holds.size?this.displayReady().then(start):start()
  const work=run.finally(()=>{if(this.pending.get(key)===work)this.pending.delete(key)})
  this.pending.set(key,work);return work
 }
 clear():void{this.epoch++;this.pending.clear();for(const release of [...this.holds])release()}
}
