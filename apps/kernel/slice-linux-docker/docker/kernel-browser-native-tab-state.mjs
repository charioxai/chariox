// MP-08/MP-10/MP-11: metadata from fresh native extension reads, reusable only
// until the owned extension/window changes. Page worlds never supply this state.
import {retireNativeStates} from './kernel-browser-native-worker.mjs';
export const nativeWindowChangedBinding='__charioxNativeWindowChanged';
export const nativeBoundsPayload='bounds';
// Events that cannot change page geometry: they drop the cached snapshot only.
const softPayloads=new Set([nativeBoundsPayload,'zoomsame']);
export class NativeTabState {
 constructor(connection,onDetached,onChange){
  this.connection=connection;this.onDetached=onDetached;this.onChange=onChange;this.supported=typeof connection.subscribe==='function';this.epoch=0;this.guardEpoch=0;this.session=null;this.ready=false;this.closed=false;this.active=null;this.viewport=null;
  this.off=null;
 }
 invalidate(){retireNativeStates();this.epoch++;this.guardEpoch++;this.active=null;this.viewport=null;this.onChange?.();}
 // A frame resize changes geometry, never which Tab is active (the extension raises the window binding for that).
 invalidateGeometry(){retireNativeStates();this.epoch++;this.guardEpoch++;this.viewport=null;this.onChange?.();}
 // A window bounds event (also the one for the resize our own native start made) drops the cached snapshot: the next
 // publication reads identity and page rect fresh and a real resize still raises Page.frameResized, which retires guards.
 invalidateBounds(){retireNativeStates();this.epoch++;this.active=null;this.viewport=null;this.onChange?.();}
 detach(){this.invalidate();this.session=null;this.ready=false;this.onDetached?.();}
 async attach(session){
  if(this.closed)throw Error('MP-11: native state observer closed');
  this.detach();this.session=session;
  if(!this.supported)return;
  this.off??=this.connection.subscribe(message=>this.onCdp(message));
  await this.connection.send('Runtime.addBinding',{name:nativeWindowChangedBinding},session);
  await this.connection.send('Runtime.enable',{},session);
  if(this.closed||this.session!==session)throw Error('MP-11: native state observer retired');
  this.ready=true;
 }
 onCdp(message){
  // Diagnostics: the CDP method that last retired this state.
  if(message?.method==='Page.frameResized'){this.cause=message.method;this.invalidateGeometry();return;}
  if(message?.method==='Chariox.browserDisconnected'){this.cause=message.method;this.detach();return;}
  if(message?.method==='Target.detachedFromTarget'&&message.params?.sessionId===this.session){this.cause=message.method;this.detach();return;}
  if(message?.sessionId!==this.session||!this.session)return;
  if(['Runtime.executionContextsCleared','Runtime.executionContextDestroyed','Inspector.detached'].includes(message.method)){this.cause=message.method;this.detach();return;}
  if(message.method==='Runtime.bindingCalled'&&message.params?.name===nativeWindowChangedBinding){this.cause=`${message.method} ${String(message.params.payload).replace(/[^a-zA-Z]/g,'').slice(0,20)}`;softPayloads.has(message.params.payload)?this.invalidateBounds():this.invalidate();}
 }
 usable(){return !this.closed&&this.ready&&!!this.session&&this.connection.isOpen?.()!==false;}
 remember(kind,value,epoch){
  if(!this.usable()||epoch!==this.epoch)return;
  if(kind==='active'){
   this.active=typeof value==='string'&&value.length>0&&value.length<=256?value:null;
   if(this.viewport?.target_id!==this.active)this.viewport=null;
  }else if(kind==='viewport'){
   const valid=value&&typeof value.target_id==='string'&&value.target_id.length>0&&value.target_id.length<=256&&[value.width,value.height].every(n=>Number.isSafeInteger(n)&&n>0&&n<=8192);
   this.viewport=valid?Object.freeze({target_id:value.target_id,width:value.width,height:value.height}):null;
   this.active=this.viewport?.target_id??null;
  }
 }
 // A caller may reuse its own observation only inside this native epoch.
 guard(){
  if(!this.usable())return null;
  const epoch=this.guardEpoch,session=this.session;
  return ()=>this.usable()&&this.guardEpoch===epoch&&this.session===session;
 }
 cachedActive(){return this.usable()?this.active:null;}
 cachedViewport(){return this.usable()?this.viewport:null;}
 close(){this.closed=true;this.detach();this.off?.();this.off=null;}
}
