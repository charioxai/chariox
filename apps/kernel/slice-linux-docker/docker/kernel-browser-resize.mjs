// MP-08/MP-10/MP-11: a mutation of the owned Tab, not a new observation attachment.
import {chromeBandHeight} from './kernel-browser-geometry.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
export async function resizeBrowserPage(host,tab,command,signal){
 const {width,height}=command;
 if(!Number.isSafeInteger(width)||!Number.isSafeInteger(height)||width<320||height<200||width>1920||height>1080)throw Error('browser resize needs a viewport between 320x200 and 1920x1080; adjust the browser area');
 assertNotCancelled(signal);
 if(host.generation!==command.generation||host.tabs.get(tab.tab_id)?.target_id!==tab.target_id)throw Error('browser resize target retired; refresh the browser view and try again');
 const previous=host.pageSize(tab.tab_id);
 const browser=host.browser,{connection,sessionId}=await browser.resolvePageTarget(tab.target_id);
 const {windowId,bounds}=await connection.send('Browser.getWindowForTarget',{targetId:tab.target_id});
 const current=()=>{assertNotCancelled(signal);if(host.browser!==browser||host.generation!==command.generation||host.tabs.get(tab.tab_id)?.target_id!==tab.target_id)throw Error('browser resize target retired; refresh the browser view and try again');};
 current();
 if(bounds?.windowState!=='normal')throw Error('restore the browser window before resizing');
 if(previous.width===width&&previous.height===height&&bounds.width===width&&bounds.height===height+chromeBandHeight)return {width,height};
 // Retire old native rectangles before moving the window. Every fresh source
 // still takes its ordinary ownership, document, policy and pixel fences.
 host.chromeWindows?.retire('viewport_resize');
 await host.closeCompositors(tab.tab_id);current();
 // The pinned Chrome toolbar has the same87-DIP launch inset. Native
 // capture independently measures and proves the resulting page rectangle.
 const outer={width,height:height+chromeBandHeight};
 await connection.send('Browser.setWindowBounds',{windowId,bounds:outer});current();
 await host.setLayout(tab,'resize',{width,height});current();
 for(const stream of host.displays.values())if(stream.tab_id===tab.tab_id&&!stream.regionTarget){stream.css_width=width;stream.css_height=height;stream.invalidate();}
 host.stateSignal.notify();
 await host.rebindWindows();current();
 return {width,height};
}
