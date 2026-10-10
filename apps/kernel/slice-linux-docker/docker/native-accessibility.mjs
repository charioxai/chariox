// MP-08 / MP-11: revision-bound opaque AT-SPI handles; the bus never crosses MCP.
import { UserDomainRefusal } from './kernel-browser-refusal.mjs';
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
    const browser_processes=await binding.browserProcesses?.();
    const raw=await this.execute({op:'accessibility',processes,...(browser_processes?{browser_processes}:{})},binding.environment,signal);
    if(signal?.aborted || this.binding()!==binding)throw new Error('MP-11: stale accessibility surface');
    const tree=redactObservation(raw,policy?.values??[]);
    return {tree,binding,processes,rawDigest:createHash('sha256').update(JSON.stringify(canonical(raw))).digest('hex'),digest:createHash('sha256').update(JSON.stringify([binding.surface_id,binding.generation,tree,canonical(policy??{})])).digest('hex')};
  }
  async snapshot(observer,policy,{signal}={}){
    const {tree,binding,digest}=await this.read(policy,signal);
    if(!tree.available){this.observers.delete(observer);return {surface_id:binding.surface_id,generation:binding.generation,tree_revision:++this.nextRevision,nodes:[],fallback:'ocr',complete:false};}
    const previous=this.observers.get(observer);
    if(previous?.digest===digest)return previous.public;
    if(this.observers.size>=64 && !previous)this.observers.delete(this.observers.keys().next().value);
    const handles=new Map();
    const revision=++this.nextRevision;
    // MP-08 / MP-11: official harnesses truncate large single-line tool output.
    // Prioritize current controls; the unprojected tree still fences every action.
    const priority=node=>node.states?.includes('showing')
      ? (node.states.includes('focused') || ['frame','window','dialog'].includes(node.role)
        ? 0 : node.actions?.length || node.states.includes('editable') ? 1 : 2) : 3;
    // MP-08 / MP-11: AT-SPI browser descendants can retain focused/showing
    // states after minimization. The private X11 selector binds the actual
    // owned foreground frame; rank its subtree before applying public limits.
    const active=tree.active_window;
    const foreground=node=>Boolean(active?.path?.length && node.pid===active.pid && node.started===active.started
      && active.path.every((part,index)=>node.path?.[index]===part));
    const candidates=(tree.nodes??[]).slice().sort((a,b)=>Number(foreground(b))-Number(foreground(a)) || priority(a)-priority(b)).slice(0,512);
    const nodes=[];let bytes=0;
    for(const node of candidates){
      if(nodes.length>=64)break;
      const target_id=`atspi-${randomUUID()}`;
      const protectedNode=Boolean(node.protected || node.role==='password text' || policy?.targets?.length);
      const projected={target_id,role:node.role,name:protectedNode?'[protected]':node.name,states:node.states??[],bounds:node.desktop_bounds??node.bounds,actions:protectedNode?[]:node.actions??[]};
      const size=Buffer.byteLength(JSON.stringify(projected))+1;
      // MP-08: MCP duplicates and pretty-prints this payload; history caps at12KiB.
      if(bytes+size>3072)continue;
      bytes+=size;nodes.push(projected);handles.set(target_id,{...node,protected:protectedNode});
    }
    const complete=Boolean(tree.complete && !tree.uncovered?.length && nodes.length===(tree.nodes??[]).length);
    // MP-08: windows without accessibility are black in captures; announce their regions (geometry only) so agents know they exist.
    const masked=(tree.masks??tree.uncovered??[]).slice(0,16);
    const result={surface_id:binding.surface_id,generation:binding.generation,tree_revision:revision,nodes,masked,complete,fallback:complete?'none':'ocr'};
    this.observers.set(observer,{digest,revision,handles,public:result});return result;
  }
  async action(observer,command,policy,{signal}={}){
    const observed=this.observers.get(observer),target=observed?.handles.get(command.target_id);
    if(!target || observed.revision!==command.tree_revision)throw new UserDomainRefusal('stale_reference');
    if(target.protected || policy?.targets?.length || !target.actions.includes(command.action))throw new UserDomainRefusal('not_granted');
    const {digest,binding,processes,rawDigest}=await this.read(policy,signal);
    if(digest!==observed.digest){this.observers.delete(observer);throw new UserDomainRefusal('stale_reference');}
    // MP-11: dispatch may apply an effect and then fail. Consume before sending.
    this.observers.delete(observer);
    const browser_processes=await binding.browserProcesses?.();
    // MP-11: the helper gates agent doAction (which may Paste) on the clipboard owner.
    const agent=Boolean(command._agent_input || observer.startsWith('agent:'));
    const result=await this.execute({op:'accessibility_action',...(agent?{agent_input:true}:{}),processes,...(browser_processes?{browser_processes}:{}),path:target.path,pid:target.pid,started:target.started,action:command.action,expected_tree_digest:rawDigest},binding.environment,signal);
    if(signal?.aborted || this.binding()!==binding)throw new Error('MP-11: accessibility action cancelled');
    return result;
  }
  retire(observer){this.observers.delete(observer);}
  clear(){this.observers.clear();}
}
