// MP-08/MP-11: bind delayed creations to an admitted source activation.
// The isolated observer only cancels evidence; it cannot grant input authority.
const WORLD = 'chariox-popup-activation';
const BINDING = 'charioxPopupNativeInput';
const OBSERVER = `(()=>{if(globalThis.charioxPopupObserver)return;globalThis.charioxPopupObserver=true;for(const event of ['pointerdown','keydown','touchstart'])addEventListener(event,e=>{if(e.isTrusted)globalThis.${BINDING}('')},true)})()`;
export class BrowserPopupEvidence {
  constructor(limit = 128) {
    this.limit = limit;
    this.targets = new Map();
    this.sources = new Map();
  }
  clear() {
    this.off?.();this.off=null;this.connection=null;
    this.targets.clear();this.sources.clear();
  }
  async source(browser, tab) {
    const connection=await browser.ensureConnection();
    if(this.connection!==connection){this.clear();this.connection=connection;this.off=connection.subscribe(message=>this.observe(message));}
    let source=this.sources.get(tab.target_id);
    if(source)return source;
    const sessionId=await browser.ensureTargetSession(connection,tab.target_id);
    const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
    await connection.send('Runtime.addBinding',{name:BINDING,executionContextName:WORLD},sessionId);
    await connection.send('Page.addScriptToEvaluateOnNewDocument',{source:OBSERVER,worldName:WORLD},sessionId);
    const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:frameTree.frame.id,worldName:WORLD},sessionId);
    await connection.send('Runtime.evaluate',{expression:OBSERVER,contextId:executionContextId},sessionId);
    source={sessionId,actionId:null,dispatches:0,expires:0,opens:[]};this.sources.set(tab.target_id,source);
    while(this.sources.size>this.limit)this.sources.delete(this.sources.keys().next().value);
    return source;
  }
  observe(message) {
    const source=[...this.sources.values()].find(s=>s.sessionId===message.sessionId);
    if(message.method==='Runtime.bindingCalled'&&message.params?.name===BINDING&&source&&!source.dispatches){source.actionId=null;source.opens=[];}
    if(message.method==='Page.windowOpen'&&source&&message.params?.userGesture&&source.actionId&&Date.now()<=source.expires){
      source.opens.push(source.actionId);if(source.opens.length>this.limit)source.opens.shift();
    }
    if(message.method!=='Target.targetCreated')return;
    const target=message.params?.targetInfo,opener=this.sources.get(target?.openerId);
    if(target?.type!=='page'||typeof target.targetId!=='string'||!opener?.actionId)return;
    // Synchronous dispatch is direct evidence; asynchronous creation must also
    // carry Chromium's retained user activation, before any later native input.
    const actionId=opener.opens.shift()??(opener.dispatches?opener.actionId:null);
    if(!actionId||Date.now()>opener.expires)return;
    this.targets.set(target.targetId,{actionId,seen:false});
    while(this.targets.size>this.limit)this.targets.delete(this.targets.keys().next().value);
  }
  async capture(browser, tab, actionId, operation) {
    if(typeof actionId!=='string'||!actionId)return operation(()=>{});
    const source=await this.source(browser,tab);
    try{return await operation(()=>{
      source.actionId=actionId;source.expires=Date.now()+5000;source.opens=[];++source.dispatches;
      return ()=>{source.dispatches=Math.max(0,source.dispatches-1);};
    });}catch(error){
      if(source.actionId===actionId){source.actionId=null;source.opens=[];source.dispatches=0;}
      for(const [target,evidence]of this.targets)if(evidence.actionId===actionId)this.targets.delete(target);
      throw error;
    }
  }
  inventory(tabs) {
    const live=new Set(tabs.map(tab=>tab.target_id));
    for(const [target,source]of this.sources){if(Date.now()>source.expires){source.actionId=null;source.opens=[];}if(source.seen&&!live.has(target))this.sources.delete(target);else if(live.has(target))source.seen=true;}
    for(const [target,evidence]of this.targets){if(live.has(target))evidence.seen=true;else if(evidence.seen)this.targets.delete(target);}
    return Object.fromEntries(tabs.flatMap(tab=>{const evidence=this.targets.get(tab.target_id);return evidence?[[tab.tab_id,evidence.actionId]]:[];}));
  }
}
