// MP-08/MP-10/MP-11: private bounded display configuration; existing protocol fields.
const selected=process.env.CHARIOX_BROWSER_DISPLAY_GEOMETRY;
if(selected && !['1280x800','1920x1080'].includes(selected))throw Error('MD-DISPLAY: unsupported canonical geometry');
export const displayGeometry=Object.freeze(selected==='1920x1080'?{width:1920,height:1080,dpr:1}:{width:1280,height:800,dpr:2});

// MP-08/MP-10/MP-11: CDP device scale sets page density; the view image
// scale fills the negotiated raster from the host window's own scale.
export const displayDeviceMetrics=(width,height,scale,hostScale=1)=>({width,height,deviceScaleFactor:scale,scale:scale/hostScale,mobile:false});

// MP-08/MP-10/MP-11: the kernel selects native page density before any
// observation/input. Viewer DPR never changes the shared browser viewport.
export function hostViewport(native = process.env.CHARIOX_KERNEL_BROWSER_DISPLAY === '1') {
 const g=displayGeometry,scale=native?g.dpr:1;
 return {css_width:g.width,css_height:g.height,device_scale_factor:scale,
  desktop_pixel_width:g.width*scale,desktop_pixel_height:g.height*scale};
}
