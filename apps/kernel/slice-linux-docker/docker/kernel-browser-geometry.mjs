// MP-08/MP-10/MP-11: private bounded display configuration; existing protocol fields.
const selected=process.env.CHARIOX_BROWSER_DISPLAY_GEOMETRY;
if(selected && !['1280x800','1920x1080'].includes(selected))throw Error('MD-DISPLAY: unsupported canonical geometry');
export const displayGeometry=Object.freeze(selected==='1920x1080'?{width:1920,height:1080,dpr:1}:{width:1280,height:800,dpr:2});

// MP-08/MP-10: CDP deviceScaleFactor selects screenshot density. The separate
// view-image scale is a page transform on the native window; keep it at unity
// while screenshot density remains a separate value.
export const displayDeviceMetrics=(width,height,scale)=>({width,height,deviceScaleFactor:scale,scale:1,mobile:false});
