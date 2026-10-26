// MP-08/MP-10/MP-11: private bounded display configuration; existing protocol fields.
const selected=process.env.CHARIOX_BROWSER_DISPLAY_GEOMETRY;
if(selected && !['1280x800','1920x1080'].includes(selected))throw Error('MD-DISPLAY: unsupported canonical geometry');
// Chrome's own tab strip + toolbar above the page (DIP).
export const chromeBandHeight=87;
export const displayGeometry=Object.freeze(selected==='1920x1080'?{width:1920,height:1080,dpr:1}:{width:1280,height:800,dpr:2});

// MP-08/MP-10: CDP deviceScaleFactor selects screenshot density. The separate
// view-image scale is a page transform on the native window; keep it at unity
// while screenshot density remains a separate value.
export const displayDeviceMetrics=(width,height,scale)=>({width,height,deviceScaleFactor:scale,scale:1,mobile:false});

// The Room browser mirror and display are on unless the kill switch is "0"; the
// kernel passes its resolved values, so these gates agree with the Rust helper.
export const featureOn=(name,environment=process.env)=>environment[name]!=='0';

// MP-08/MP-10/MP-11: the kernel selects native page density before any
// observation/input. Viewer DPR never changes the shared browser viewport.
export function hostViewport(native = featureOn('CHARIOX_KERNEL_BROWSER_MIRROR') || featureOn('CHARIOX_KERNEL_BROWSER_DISPLAY')) {
 const g=displayGeometry,scale=native?g.dpr:1;
 return {css_width:g.width,css_height:g.height,device_scale_factor:scale,
  desktop_pixel_width:g.width*scale,desktop_pixel_height:g.height*scale};
}
