// MP-08/MP-10/MP-11: validate owned paths before reading state; no default HOME.
export function probePaths(root,evidence){
 if(typeof root!=='string'||typeof evidence!=='string'||root.includes('..')||evidence.includes('..')||
  !/^\/root\/\.chariox\/dev\/vidlat\/(?:r\d+\/drills\/[\w-]+|owner-ab-r\d+-[\w-]+)$/.test(root)||
  !/^\/root\/\.codex\/evidence\/browser-resume-20260930\/vidlat\/r\d+\/[\w/-]+$/.test(evidence))throw Error('MP-11 owned probe roots required');
 const ab=root.includes('/owner-ab-');
 return {runtime:root+(ab?'/V/runtime':'/runtime'),V:root+(ab?'/V':''),S:root+(ab?'/S':''),viewer:root+'/viewer',tmp:root+'/tmp',ownerAB:ab};
}
export function typingMetric(rows){
 return {attempts:rows.length,samples:rows.filter(r=>(r.source_echo_matches??r.source_equals_public_c)===true)
  .map(r=>r.physical?.latency_ms).filter(Number.isFinite)};
}

// MP-10: CDP lists new/background tabs first. Observe visibility only; never
// activate a source target to make a coordinate oracle appear correct.
export async function visibleSourcePage(pages,isVisible){
 const visible=[];for(const page of pages)if(page.type==='page'&&await isVisible(page))visible.push(page);
 if(visible.length!==1)throw Error(visible.length?'MP-10 ambiguous visible source':'MP-10 missing visible source');
 return visible[0];
}
