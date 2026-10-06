// MD-DISPLAY-02/04: a new document requires an independent negotiated image.
export function assertIndependentNavigation(frame,previousDocument,codec) {
 // MP-11: PNG is always negotiated and is the independent, exact fallback
 // when the protected codec guard rejects a lossy reconstruction.
 const allowed=frame?.kind==='png'||codec!=='png'&&['video','stripes'].includes(frame?.kind);
 if(!frame||frame.document_id===previousDocument||!allowed||(frame.kind==='video'&&frame.key!==true)||(frame.kind==='stripes'&&(!frame.stripes.every(r=>r.key)||frame.stripes.length!==8)))throw Error('MD-DISPLAY: navigation did not deliver fresh independent frame');
}
