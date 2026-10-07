// MP-08/MP-10/MP-11: private bounded display configuration; existing protocol fields.
const selected=process.env.CHARIOX_BROWSER_DISPLAY_GEOMETRY;
if(selected && !['1280x800','1920x1080'].includes(selected))throw Error('MD-DISPLAY: unsupported canonical geometry');
export const displayGeometry=Object.freeze(selected==='1920x1080'?{width:1920,height:1080,dpr:1}:{width:1280,height:800,dpr:2});

// Physical font scaling is independent of negotiated raster density. A DPR2
// system-font scaler creates half-pixel metrics which a DPR1 viewer rounds
// differently. Keep native font geometry integral; Emulation scales raster and
// trusted pointer coordinates to the subscriber's device pixels. No wire field.
export const hostDisplayScale=1;
