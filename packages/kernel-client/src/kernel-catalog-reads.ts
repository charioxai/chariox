// MP-08/MP-10/MP-11: one shared policy for ordinary and hosted catalog reads.
// Scope belongs to a transport/authority generation. No persisted cache, no
// mutation sharing, and no catalog or credential values enter diagnostics.
export class KernelCatalogReads {
 private readonly pending=new Map<string,Promise<unknown>>()
 run<T>(request:unknown,scope:string,send:()=>Promise<T>):Promise<T>{
  if(!request||typeof request!=='object'||Array.isArray(request)||Object.keys(request).length!==1||!Object.hasOwn(request,'GetProviderCatalog'))return send()
  const key=JSON.stringify([scope,request]),existing=this.pending.get(key)
  if(existing)return existing as Promise<T>
  if(this.pending.size>=32)return send()
  const work=send().finally(()=>{if(this.pending.get(key)===work)this.pending.delete(key)})
  this.pending.set(key,work);return work
 }
 clear():void{this.pending.clear()}
}
