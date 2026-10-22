// MP-08/MP-10/MP-11: private bounded display configuration; existing protocol fields.
const selected=process.env.CHARIOX_BROWSER_DISPLAY_GEOMETRY;
if(selected && !['1280x800','1920x1080'].includes(selected))throw Error('MD-DISPLAY: unsupported canonical geometry');
// Chrome's own tab strip + toolbar above the page (DIP).
export const chromeBandHeight=87;
export const displayGeometry=Object.freeze(selected==='1920x1080'?{width:1920,height:1080,dpr:1}:{width:1280,height:800,dpr:2});
// Chrome's own window frame around the page (DIP, measured on the owned desktop: 4 px a side, 5 px below).
export const windowFrame=Object.freeze({width:8,height:5});
// The owned window and the Xvfb screen that holds it: band + frame + a page area equal to the layout, so
// `position:fixed` bottom UI is never cut off by a smaller window.
export const ownedWindowSize=Object.freeze({width:displayGeometry.width+windowFrame.width,height:displayGeometry.height+chromeBandHeight+windowFrame.height});
// Chrome shaves one pixel off a window that fills its screen exactly (odd X rows the band capture refuses),
// so the owned screen is a little larger than the window.
export const ownedScreenSize=Object.freeze({width:ownedWindowSize.width+16,height:ownedWindowSize.height+16});

// MP-08/MP-10: CDP deviceScaleFactor selects screenshot density. The separate
// view-image scale is a page transform on the native window; keep it at unity
// while screenshot density remains a separate value.
export const displayDeviceMetrics=(width,height,scale)=>({width,height,deviceScaleFactor:scale,scale:1,mobile:false});

// MP-08/MP-10/MP-11: the kernel selects native page density before any
// observation/input. Viewer DPR never changes the shared browser viewport.
export function hostViewport(native = process.env.CHARIOX_KERNEL_BROWSER_MIRROR === '1' || process.env.CHARIOX_KERNEL_BROWSER_DISPLAY === '1') {
 const g=displayGeometry,scale=native?g.dpr:1;
 return {css_width:g.width,css_height:g.height,device_scale_factor:scale,
  desktop_pixel_width:g.width*scale,desktop_pixel_height:g.height*scale};
}
