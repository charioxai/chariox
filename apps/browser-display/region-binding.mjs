// MP-08/MP-10/MP-11: only a negotiated kernel lease admits crop geometry.
export const regionMinimumProtocolVersion = 519;
export const visiblePageMinimumProtocolVersion = 536;
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
  frame.device_scale_factor===binding.device_scale_factor&&frame.document_id===binding.document_id;
 return (frame.css_width===1280&&frame.css_height===800)||
  (frame.css_width===1920&&frame.css_height===1080&&frame.device_scale_factor===1);
}
export function regionInputBinding(binding) {
 if(binding.source_kind!=='region')return {};
 if(!regionBindingCurrent(binding))throw Error('MD-DISPLAY: invalid geometry/binding');
 return {binding_id:binding.binding_id,sequence:binding.sequence};
}
