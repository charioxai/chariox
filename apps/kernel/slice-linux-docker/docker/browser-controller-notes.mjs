import { withBrowserFrames } from "./browser-controller-frames.mjs";
import { observationProtectedVariants } from "./browser-controller-snapshot.mjs";
// MD-N2 / MP-08 / MP-11: runs ONLY in the #607 controller isolated world.
// No DOM overlay, main-world binding, postMessage, or page-visible note data.
export const NOTE_OBSERVER_EXPRESSION = `(${installNoteObserver.toString()})()`;
// Runs on a frame owner resolved into the parent's isolated world.
const OWNER_PROTECTED_FUNCTION = `function() {
  for (let current=this;current;current=current.getRootNode?.()?.host) {
    if (current.closest('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')) return true;
  }
  return false;
}`;

function installNoteObserver() {
  if (globalThis.__charioxNotes) return true;
  const maxText = 2 * 1024 * 1024;
  // MP-08/MP-10/MP-11: ordinary ad-heavy pages exceed 64 open roots.
  // Keep a finite complete search, including late contextual/ambiguous matches.
  const maxRoots = 256;
  const roots = () => {
    const result=[document.body??document.documentElement];
    for (let i=0;i<result.length;i++) {
      const walker=document.createTreeWalker(result[i],NodeFilter.SHOW_ELEMENT);
      for (let node=walker.nextNode();node;node=walker.nextNode()) if (node.shadowRoot) {
        if (result.length>=maxRoots) throw new Error('MD-N2: shadow root limit reached');
        result.push(node.shadowRoot);
      }
    }
    return result;
  };
  // A shadow tree inherits the observation boundary of every shadow host.
  // Isolated-world DOM methods cannot be replaced by page scripts.
  const protectedElement = element => {
    for (let current=element;current;current=current.getRootNode?.()?.host) {
      if (current.closest('script,style,noscript,input,textarea,[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')) return true;
    }
    return false;
  };
  const index = (root=document.body??document.documentElement) => {
    const nodes = [];
    let text = "";
    if (root instanceof ShadowRoot && protectedElement(root.host)) return {nodes,text};
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (protectedElement(node.parentElement)) continue;
      const range = document.createRange(); range.selectNodeContents(node);
      if (!range.getClientRects().length) continue;
      if (text.length + node.length > maxText) return null;
      nodes.push({ node, start: text.length }); text += node.data.toWellFormed();
    }
    return { nodes, text };
  };
  const box = range => {
    const r = range.getBoundingClientRect();
    return r.width > 0 && r.height > 0 ? { x:r.x, y:r.y, width:r.width, height:r.height } : null;
  };
  const offset = (nodes, container, value) => {
    let result = 0;
    const point = document.createRange(); point.setStart(container, value); point.collapse(true);
    for (const {node,start} of nodes) {
      if (node === container) return start + value;
      const end = document.createRange(); end.selectNodeContents(node); end.collapse(false);
      if (point.compareBoundaryPoints(Range.START_TO_START, end) >= 0) result = start + node.length;
      else break;
    }
    return result;
  };
  let protectedVariants = [];
  const contextBoundary = (text, at, direction) => {
    for (let i=0;i<64 && (direction<0?at>0:at<text.length);i++) {
      if (direction>0) at+=text.codePointAt(at)>0xffff?2:1;
      else {
        at--;
        if (at>0 && text.charCodeAt(at)>=0xdc00 && text.charCodeAt(at)<=0xdfff
            && text.charCodeAt(at-1)>=0xd800 && text.charCodeAt(at-1)<=0xdbff) at--;
      }
    }
    return at;
  };
  const protectedSpan = (text, start, end) => protectedVariants.some(value => {
    // Search the unsplit index, including values beginning before the output
    // window or ending beyond it. Exact/context boundaries cannot hide them.
    const at=text.indexOf(value,Math.max(0,start-value.length+1));
    return at>=0 && at<end;
  });
  const capture = (variants=protectedVariants) => {
    protectedVariants=variants;
    const selection = document.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount !== 1) return null;
    let range = selection.getRangeAt(0);
    if (typeof selection.getComposedRanges==='function') {
      const composed=selection.getComposedRanges({shadowRoots:roots().filter(root=>root instanceof ShadowRoot)});
      if (composed.length!==1) return null;
      range=document.createRange();range.setStart(composed[0].startContainer,composed[0].startOffset);range.setEnd(composed[0].endContainer,composed[0].endOffset);
    }
    const root=range.startContainer.getRootNode();
    if (root!==range.endContainer.getRootNode()) return null;
    const indexed = index(root instanceof ShadowRoot?root:document.body??document.documentElement); if (!indexed) return null;
    const start = offset(indexed.nodes, range.startContainer, range.startOffset);
    const end = offset(indexed.nodes, range.endContainer, range.endOffset);
    const before=contextBoundary(indexed.text,start,-1), after=contextBoundary(indexed.text,end,1);
    if (protectedSpan(indexed.text,before,after)) return null;
    const exact = indexed.text.slice(start,end);
    if (!exact.isWellFormed() || !exact.trim() || new TextEncoder().encode(exact).length > 16384) return null;
    const rect = box(range); if (!rect) return null;
    return { quote: { exact, prefix:indexed.text.slice(before,start).toWellFormed(), suffix:indexed.text.slice(end,after).toWellFormed() }, box_css:rect, hint:JSON.stringify({start,end}) };
  };
  document.addEventListener('selectionchange',()=>capture());
  new MutationObserver(()=>{ capture(); }).observe(document.documentElement,{subtree:true,childList:true,characterData:true});
  globalThis.__charioxNotes=Object.freeze({capture});
  return true;
}

// MP-08/MP-10/MP-11: CDP descriptions may contain page values; carry only
// closed exception classes and positions inside our shipped observer function.
function noteObservationFailure(result) {
  const details=result?.exceptionDetails, exception=details?.exception;
  const classes=new Map([['TypeError','type_error'],['ReferenceError','reference_error'],['RangeError','range_error'],['SyntaxError','syntax_error'],['DOMException','dom_exception']]);
  const known=new Map([['Error: MD-N2: shadow root limit reached','note_shadow_root_limit'],['Error: MD-N2: page text limit reached','note_text_limit']]);
  const first=typeof exception?.description==='string'?exception.description.split('\n',1)[0]:'';
  const code=!details?'note_observer_result_unavailable':known.get(first)??('note_observer_'+(classes.get(exception?.className)??'exception'));
  const error=Object.assign(Error('MD-N2: selection observation failed'),{code});
  const names=new Set(['capture','Object.capture','roots','index','offset','contextBoundary','protectedSpan','box','matchQuote','reanchor','Object.reanchor']);
  const frames=details?.stackTrace?.callFrames;
  const frame=Array.isArray(frames)?frames.slice(0,32).find(frame=>names.has(frame.functionName)&&Number.isSafeInteger(frame.lineNumber)&&frame.lineNumber>=0&&frame.lineNumber<installNoteObserver.toString().split('\n').length):undefined;
  if(frame)error.noteObserverLine=frame.lineNumber+1;
  return error;
}

export async function observeBrowserNote(browser, request) {
  if (typeof request?.target_id !== 'string' || typeof request?.document_id !== 'string') throw new Error('MD-N2: observed target/document required');
  const connection=await browser.ensureConnection();
  const protectedVariants=observationProtectedVariants(browser.protectedValues??[]);
  const session=await browser.ensureTargetSession(connection,request.target_id);
  const readFrame=async()=> (await connection.send('Page.getFrameTree',{},session)).frameTree?.frame;
  const frame=await readFrame();
  if (!frame || frame.loaderId!==request.document_id) throw new Error('MD-N2: stale document; refresh tab');
  return withBrowserFrames(connection,session,request.target_id,request.document_id,async entries=>{
    // Reuse the controller's frame ownership channel, including OOPIF renderers.
    const frames=new Map();
    for (const entry of entries) {
      const visit=(tree,parent)=>{
        const item={frame:tree.frame,sessionId:entry.sessionId,parent};
        frames.set(tree.frame.id,item);
        for (const child of tree.childFrames??[]) visit(child,item);
      };
      visit(entry.tree,entry.frame.parentId?frames.get(entry.frame.parentId):null);
    }
    // OOPIF entries replace a parent's placeholder with the owning session.
    for (const item of frames.values()) if (item.frame.parentId) item.parent=frames.get(item.frame.parentId);
    // A frame inherits protection from its owner element, the owner's
    // ancestors and shadow hosts, and every enclosing frame. Unknown ownership
    // is protected.
    const exposed=new Map();
    const frameExposed=item=>{
      if (!item.frame.parentId) return Promise.resolve(item.frame.id===frame.id);
      if (!exposed.has(item)) exposed.set(item,(async()=>{
        if (!item.parent || !await frameExposed(item.parent)) return false;
        const {sessionId}=item.parent;
        const owner=await connection.send('DOM.getFrameOwner',{frameId:item.frame.id},sessionId);
        const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:item.parent.frame.id,worldName:'chariox-controller-focus',grantUniveralAccess:false},sessionId);
        const {object}=await connection.send('DOM.resolveNode',{backendNodeId:owner.backendNodeId,executionContextId},sessionId);
        try {
          const result=await connection.send('Runtime.callFunctionOn',{objectId:object.objectId,functionDeclaration:OWNER_PROTECTED_FUNCTION,returnByValue:true},sessionId);
          return !result.exceptionDetails && result.result?.value===false;
        } finally { await connection.send('Runtime.releaseObject',{objectId:object.objectId},sessionId).catch(()=>{}); }
      })().catch(()=>false));
      return exposed.get(item);
    };
    const worlds=[];
    for (const item of frames.values()) {
      if (!item.frame.loaderId || !await frameExposed(item)) continue;
      const world=item.frame.id===frame.id
        ? await browser.ensureFocusWorld(connection,session,request.target_id,frame)
        : {contextId:(await connection.send('Page.createIsolatedWorld',{frameId:item.frame.id,worldName:'chariox-controller-focus',grantUniveralAccess:false},item.sessionId)).executionContextId};
      if (!Number.isSafeInteger(world.contextId) || world.contextId<=0) throw new Error('MD-N2: isolated world unavailable');
      if (!world.notesInstalled) {
        const installed=await connection.send('Runtime.evaluate',{expression:NOTE_OBSERVER_EXPRESSION,contextId:world.contextId,returnByValue:true},item.sessionId);
        if (installed.exceptionDetails || installed.result?.value!==true) throw new Error('MD-N2: isolated observer unavailable');
        world.notesInstalled=true;
      }
      worlds.push({item,world});
    }
    const candidates=[];
    {
      for (const {item,world} of worlds) {
        const expression=`globalThis.__charioxNotes.capture(${protectedVariants.length?JSON.stringify(protectedVariants):''})`;
        const result=await connection.send('Runtime.evaluate',{expression,contextId:world.contextId,returnByValue:true,awaitPromise:false},item.sessionId);
        if (result.exceptionDetails || !Object.hasOwn(result.result??{},'value')) throw noteObservationFailure(result);
        const value=result.result.value;
        if (!value) continue;
        if (value.box_css) value.box_css=await projectFrameBox(connection,item,value.box_css);
        if (value.hint) value.hint=JSON.stringify({frame_id:item.frame.id,document_id:item.frame.loaderId,range:value.hint});
        candidates.push({value,url:item.frame.url,item});
      }
    }
    // Recheck the documents that own released text and their enclosing frames.
    // Unrelated advertising frames may change while the selected document stays
    // current; comparing the whole tree makes ordinary selection polling fail.
    const required=new Set([frame.id]);
    for (const candidate of candidates) for (let item=candidate.item;item;item=item.parent) required.add(item.frame.id);
    const identities=tree=>{
      const result=new Map();
      const visit=tree=>{
        if(required.has(tree.frame.id))result.set(tree.frame.id,[tree.frame.loaderId,tree.frame.parentId,tree.frame.url]);
        for(const child of tree.childFrames??[])visit(child);
      };
      visit(tree);return result;
    };
    for (const entry of entries) {
      const before=identities(entry.tree);
      if(!before.size)continue;
      const current=await connection.send('Page.getFrameTree',{},entry.sessionId);
      if(!current.frameTree?.frame)throw new Error('MD-N2: frame changed during capture');
      const after=identities(current.frameTree);
      for(const [id,identity]of before)if(JSON.stringify(after.get(id))!==JSON.stringify(identity))throw new Error('MD-N2: frame changed during capture');
    }
    // Owner protection may have changed independently of the frame tree.
    exposed.clear();
    for(const candidate of candidates)if(!await frameExposed(candidate.item))throw new Error('MD-N2: frame changed during capture');
    const after=await readFrame();
    if (after?.loaderId!==frame.loaderId) throw new Error('MD-N2: document changed during capture');
    const chosen=candidates.length===1?candidates[0]:null;
    return { target_id:request.target_id,document_id:frame.loaderId,url:chosen?.url??frame.url,
      selection:chosen?.value??null };
  });
}

async function projectFrameBox(connection,item,box) {
  if (!item.parent) return box;
  const owner=await connection.send('DOM.getFrameOwner',{frameId:item.frame.id},item.parent.sessionId);
  const {model}=await connection.send('DOM.getBoxModel',{backendNodeId:owner.backendNodeId},item.parent.sessionId);
  const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:item.frame.id,worldName:'chariox-controller-focus',grantUniveralAccess:false},item.sessionId);
  if (!Number.isSafeInteger(executionContextId)||executionContextId<=0) throw new Error('MD-N2: isolated frame geometry unavailable');
  const viewport=await connection.send('Runtime.evaluate',{expression:'({width:innerWidth,height:innerHeight})',contextId:executionContextId,returnByValue:true},item.sessionId);
  if (viewport.exceptionDetails) throw new Error('MD-N2: isolated frame geometry unavailable');
  const size=viewport.result?.value,quad=model?.content;
  if (!quad || quad.length!==8 || !size || !(size.width>0 && size.height>0)) throw new Error('MD-N2: frame geometry unavailable');
  const points=[];
  for (const [x,y] of [[box.x,box.y],[box.x+box.width,box.y],[box.x+box.width,box.y+box.height],[box.x,box.y+box.height]]) {
    points.push([quad[0]+(quad[2]-quad[0])*x/size.width+(quad[6]-quad[0])*y/size.height,quad[1]+(quad[3]-quad[1])*x/size.width+(quad[7]-quad[1])*y/size.height]);
  }
  const xs=points.map(p=>p[0]),ys=points.map(p=>p[1]);
  return projectFrameBox(connection,item.parent,{x:Math.min(...xs),y:Math.min(...ys),width:Math.max(...xs)-Math.min(...xs),height:Math.max(...ys)-Math.min(...ys)});
}
