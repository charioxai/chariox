// MD-DISPLAY-02/04: a new document requires an independent negotiated image.
export function assertIndependentNavigation(frame,previousDocument,codec) {
 const expected=codec==='png'?'png':'video';
 if(!frame||frame.document_id===previousDocument||!(frame.kind===expected||expected==='video'&&frame.kind==='stripes')||(frame.kind==='video'&&frame.key!==true)||(frame.kind==='stripes'&&(!frame.stripes.every(r=>r.key)||frame.stripes.length!==8)))throw Error('MD-DISPLAY: navigation did not deliver fresh independent frame');
}
