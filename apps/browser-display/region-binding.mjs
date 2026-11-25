// MP-08/MP-10/MP-11: only a negotiated kernel lease admits crop geometry.
export const regionMinimumProtocolVersion = 519;
export const visiblePageMinimumProtocolVersion = 536;
// MP-08/MP-10/MP-11 (P1): a Tab display carries the Tab's later documents on the same binding.
export const tabDisplayMinimumProtocolVersion = 720;
// MP-08/MP-11 (row 341): the whole-page video's still at rest may come at the viewer's own density. Asked for only of a
// kernel that advertises this capability in its RelayStatus: a protocol number alone can name another kernel's work.
export const viewerDensityStillsCapability = 'viewer_density_stills_v1';
// Row 487: a visible page follows a resize of the Browser window in the same subscription. Asked for only of a kernel that
// advertises this capability in its RelayStatus (like viewer-density stills).
export const resizeRebindCapability = 'display_resize_rebind_v1';
/** The binding a resize_rebind frame names: the same crop binding at a later window epoch and its new size, or null. */
export function reboundBinding(binding,frame) {
 if(binding.resize_rebind!==true||binding.source_kind!=='region'||!Number.isSafeInteger(frame.binding_sequence)||frame.binding_sequence<=binding.sequence||
  !(frame.device_scale_factor===binding.device_scale_factor||frame.kind==='png'&&frame.device_scale_factor===binding.viewer_scale))return null;
 const next={...binding,sequence:frame.binding_sequence,css_width:frame.css_width,css_height:frame.css_height};
 return regionBindingCurrent(next)?next:null;
}
export function regionBindingCurrent(binding) {
 const {binding_id,document_id,sequence,css_width,css_height,device_scale_factor:scale}=binding;
 return binding.source_kind==='region'&&typeof binding_id==='string'&&binding_id.length>0&&binding_id.length<=256&&
  typeof document_id==='string'&&document_id.length>0&&document_id.length<=256&&
  Number.isSafeInteger(sequence)&&sequence>0&&[1,2].includes(scale)&&
  Number.isSafeInteger(css_width)&&Number.isSafeInteger(css_height)&&
  css_width>=1&&css_width*scale<=2560&&css_height>=1&&css_height*scale<=1600&&
  (css_width*scale)%2===0&&(css_height*scale)%2===0;
}
export function displayGeometryMatches(binding,frame) {
 const source=binding.source;
 if(source?.kind==='desktop')return frame.document_id===source.generation&&frame.device_scale_factor===binding.device_scale_factor&&frame.width===source.width&&frame.height===source.height&&frame.css_width===source.width/frame.device_scale_factor&&frame.css_height===source.height/frame.device_scale_factor;
 if(binding.source_kind==='region')return regionBindingCurrent(binding)&&
  frame.css_width===binding.css_width&&frame.css_height===binding.css_height&&
  (frame.device_scale_factor===binding.device_scale_factor||frame.kind==='png'&&frame.device_scale_factor===binding.viewer_scale)&&
  (binding.tab_display===true||frame.document_id===binding.document_id);
 return (frame.css_width===1280&&frame.css_height===800)||
  (frame.css_width===1920&&frame.css_height===1080&&frame.device_scale_factor===1);
}
export function regionInputBinding(binding) {
 if(binding.source_kind!=='region')return {};
 if(!regionBindingCurrent(binding))throw Error('MD-DISPLAY: invalid geometry/binding');
 return {binding_id:binding.binding_id,sequence:binding.sequence};
}
