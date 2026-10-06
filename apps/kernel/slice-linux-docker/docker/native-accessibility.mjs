// MP-08 / MP-11: revision-bound opaque AT-SPI handles; the bus never crosses MCP.
import { createHash, randomUUID } from 'node:crypto';
import { executeNative } from './native-computer.mjs';
import { redactObservation } from './browser-controller-snapshot.mjs';
const canonical=value=>Array.isArray(value)?value.map(canonical):value && typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])):value;
export class NativeAccessibility {
  constructor({binding,execute=executeNative}){this.binding=binding;this.execute=execute;this.observers=new Map();this.nextRevision=0;}
  async read(policy,signal){
    const binding=this.binding();
    if(!binding || policy?.unknown)throw new Error('MP-11: accessibility protection unavailable');
    const processes=await binding.ownedProcesses?.();
    if(!processes)throw new Error('MP-11: accessibility app scope unavailable');
    const raw=await this.execute({op:'accessibility',processes},binding.environment,signal);
    if(signal?.aborted || this.binding()!==binding)throw new Error('MP-11: stale accessibility surface');
    const tree=redactObservation(raw,policy?.values??[]);
    return {tree,binding,processes,rawDigest:createHash('sha256').update(JSON.stringify(canonical(raw))).digest('hex'),digest:createHash('sha256').update(JSON.stringify([binding.surface_id,binding.generation,tree])).digest('hex')};
  }
  async snapshot(observer,policy,{signal}={}){
    const {tree,binding,digest}=await this.read(policy,signal);
    if(!tree.available){this.observers.delete(observer);return {surface_id:binding.surface_id,generation:binding.generation,tree_revision:++this.nextRevision,nodes:[],fallback:'ocr',complete:false};}
    const previous=this.observers.get(observer);
    if(previous?.digest===digest)return previous.public;
    if(this.observers.size>=64 && !previous)this.observers.delete(this.observers.keys().next().value);
    const handles=new Map();
    const revision=++this.nextRevision;
    const nodes=(tree.nodes??[]).slice(0,512).map(node=>{
      const target_id=`atspi-${randomUUID()}`;
      const protectedNode=Boolean(node.protected || node.role==='password text' || policy?.targets?.length);
      handles.set(target_id,{...node,protected:protectedNode});
      return {target_id,role:node.role,name:protectedNode?'[protected]':node.name,states:node.states??[],bounds:node.bounds,actions:protectedNode?[]:node.actions??[]};
    });
    const result={surface_id:binding.surface_id,generation:binding.generation,tree_revision:revision,nodes,complete:tree.complete,fallback:tree.complete?'none':'ocr'};
    this.observers.set(observer,{digest,revision,handles,public:result});return result;
  }
  async action(observer,command,policy,{signal}={}){
    const observed=this.observers.get(observer),target=observed?.handles.get(command.target_id);
    if(!target || observed.revision!==command.tree_revision || target.protected || !target.actions.includes(command.action))throw new Error('MP-11: inaccessible or foreign target');
    const {digest,binding,processes,rawDigest}=await this.read(policy,signal);
    if(digest!==observed.digest){this.observers.delete(observer);throw new Error('MP-11: stale accessibility target; rediscover');}
    const result=await this.execute({op:'accessibility_action',processes,path:target.path,pid:target.pid,started:target.started,action:command.action,expected_tree_digest:rawDigest},binding.environment,signal);
    // Handles cannot be reused after a potentially mutating accessibility action.
    this.observers.delete(observer);
    if(signal?.aborted || this.binding()!==binding)throw new Error('MP-11: accessibility action cancelled');
    return result;
  }
  retire(observer){this.observers.delete(observer);}
  clear(){this.observers.clear();}
}
