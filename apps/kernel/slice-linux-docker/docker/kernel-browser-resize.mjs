// MP-08/MP-10/MP-11: a mutation of the owned Tab, not a new observation attachment.
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {withWindowGeometry} from './kernel-browser-window-geometry.mjs';
import {pageLimit} from './kernel-browser-geometry.mjs';
export async function resizeBrowserPage(host,tab,command,signal){
 const {width,height}=command;
 if(!Number.isSafeInteger(width)||!Number.isSafeInteger(height)||width<320||height<200||width>pageLimit.width||height>pageLimit.height)throw Error('browser resize needs a viewport between 320x200 and 1920x1080; adjust the browser area');
 if(width%2||height%2)throw Error('browser resize needs even width and height for video; round down to an even viewport size');
 assertNotCancelled(signal);
 if(host.generation!==command.generation||host.tabs.get(tab.tab_id)?.target_id!==tab.target_id)throw Error('browser resize target retired; refresh the browser view and try again');
 const previous=host.pageSize(tab.tab_id);
 const browser=host.browser,{connection,sessionId}=await browser.resolvePageTarget(tab.target_id);
 const current=()=>{assertNotCancelled(signal);if(host.browser!==browser||host.generation!==command.generation||host.tabs.get(tab.tab_id)?.target_id!==tab.target_id)throw Error('browser resize target retired; refresh the browser view and try again');};
 current();
 const measure=async()=>{
 current();
 let page;
 if(host.chromium?.desktop?.binding?.()){
  // A native viewport resize can leave the page binding valid but its rectangle stale.
  // Refresh it before combining it with the current CDP window bounds.
  await host.chromeWindows.resolve();current();
  const window=host.chromeWindows.current().tabbed;
  page={width:window.page_rect.width/window.scale,height:window.page_rect.height/window.scale};
 }else{
  const metrics=await connection.send('Page.getLayoutMetrics',{},sessionId);page={width:metrics.cssLayoutViewport?.clientWidth,height:metrics.cssLayoutViewport?.clientHeight};
 }
 const {windowId,bounds}=await connection.send('Browser.getWindowForTarget',{targetId:tab.target_id});current();
 if(bounds?.windowState!=='normal')throw Error('restore the browser window before resizing');
 const inset={width:bounds.width-page.width,height:bounds.height-page.height};
 if(![inset.width,inset.height].every(Number.isSafeInteger)||inset.width<0||inset.width>64||inset.height<0||inset.height>256)throw Error('browser window geometry unavailable; refresh the browser view and resize again');
 const outer={width:width+inset.width,height:height+inset.height};
 return {windowId,bounds,outer};
 };
 // Capture fits share the window, including while display credits run beside input.
 const measured=await withWindowGeometry(connection,measure);
 const complete=!host.windowLayout||host.resizeSettled===host.windowLayout;
 if(complete&&previous.width===width&&previous.height===height&&measured.bounds.width===measured.outer.width&&measured.bounds.height===measured.outer.height)return {width,height};
 host.chromeWindows?.retire('viewport_resize');
 // A compositor may be waiting for a fit. Drain it outside the geometry lane.
 await host.closeCompositors();current();
 return withWindowGeometry(connection,async()=>{
 const {windowId,outer}=await measure();
 await connection.send('Browser.setWindowBounds',{windowId,bounds:outer});current();
 // Every Tab shares the one window: all of them, and Tabs opened later, are laid out at its size.
 const layout=host.windowLayout={width,height};
 for(const other of [...host.tabs.values()]){await host.setLayout(other,'resize',{width,height});current();}
 for(const stream of host.displays.values())if(!stream.regionTarget){stream.css_width=width;stream.css_height=height;stream.invalidate(true,'resize');}
 host.stateSignal.notify();
 await host.rebindWindows();current();
 host.resizeSettled=layout;
 await host.save?.();
 return {width,height};
 });
}
