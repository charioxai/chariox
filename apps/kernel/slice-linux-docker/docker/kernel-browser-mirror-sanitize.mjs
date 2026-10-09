// MP-08/MP-11: the final structured tree, hash and incremental base share one policy.
// Ordinary matching pixels remain visible through the protected compositor path.
import { observationProtectedVariants } from './browser-controller-snapshot.mjs';

export function sanitizeMirrorTree(source, values) {
  const variants=observationProtectedVariants(values);
  if(!variants.length)return false;
  const matches=value=>typeof value==='string'&&variants.some(variant=>value.includes(variant));
  const contains=value=>typeof value==='string'?matches(value):value&&typeof value==='object'&&Object.values(value).some(contains);
  const byId=new Map(source.nodes.map(node=>[node.id,node])),tiles=new Set();
  // A scrubbed style/font can change layout outside its own node. Use the
  // existing full compositor fallback rather than an invented CSS replacement.
  if(source.fonts.some(contains)||source.nodes.some(node=>contains(node.style)))return true;
  for(const node of source.nodes) {
    if(!matches(node.text)&&!contains(node.attributes)&&!contains(node.form)&&!contains(node.pseudo))continue;
    let target=node;
    while(target&&!['element','tile','mask'].includes(target.kind))target=byId.get(target.parent);
    if(!target?.box)return true;
    // An overflowing text run needs more than the containing element's crop.
    if(node.kind==='text'&&node.box&&(node.box.x<target.box.x-.5||node.box.y<target.box.y-.5||node.box.x+node.box.width>target.box.x+target.box.width+.5||node.box.y+node.box.height>target.box.y+target.box.height+.5))return true;
    tiles.add(target);
  }
  for(const node of tiles) {
    // Keep layout geometry and safe sampled CSS, but no page text/attributes.
    // Descendants are removed by the service's normal opaque-subtree pass.
    node.kind='tile';node.reason='protected_text';
    delete node.text;delete node.attributes;delete node.form;delete node.pseudo;delete node.resource;
  }
  return false;
}
