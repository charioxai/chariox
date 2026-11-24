// MP-08/MP-10/MP-11: private bounded display configuration; existing protocol fields.
const selected=process.env.CHARIOX_BROWSER_DISPLAY_GEOMETRY;
if(selected && !['1280x800','1920x1080'].includes(selected))throw Error('MD-DISPLAY: unsupported canonical geometry');
// Chrome's own tab strip + toolbar above the page (DIP).
export const chromeBandHeight=87;
export const displayGeometry=Object.freeze(selected==='1920x1080'?{width:1920,height:1080,dpr:1}:{width:1280,height:800,dpr:2});
// BH: the largest page a live resize lays out (CSS px), per side and in area. A headless browser: 3840 per side, 3840x2400.
// An owned Linux display (headed) lays the page out at scale 1, where its video path carries 2560 per side, 2560x1600.
// Page rasters of a resized Tab are bounded by the larger one at the Tab's density.
export const pageBound=Object.freeze({headless:Object.freeze({side:3840,area:3840*2400}),headed:Object.freeze({side:2560,area:2560*1600})});
export const pageLimit=pageBound.headless;
// The page a kernel lays out for a wanted size: that size when it fits the bound, else the largest even size of its shape
// inside it (the viewer scales it up to the pane: never a band, never another layout width).
export function boundedPage(width,height,bound){
 const k=Math.min(1,bound.side/width,bound.side/height,Math.sqrt(bound.area/(width*height)));
 return k===1?{width,height}:{width:Math.max(320,Math.floor(width*k/2)*2),height:Math.max(200,Math.floor(height*k/2)*2)};
}
// The headed video path's raster bound (device px): 2560 per side, 2560x1600 in area.
export const videoFits=(width,height)=>width>=1&&height>=1&&width<=2560&&height<=2560&&width*height<=2560*1600;

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
