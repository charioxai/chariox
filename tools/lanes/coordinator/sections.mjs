// MP-08 / MP-09 / MP-10 / MP-11: the one place that says where a coordinator note's sections begin.
// The inbox consumer and the direct steer must cut a note at the same headings, or a note delivered directly is delivered again.
export function newSections(text, offset) {
  const tail = Buffer.from(text).subarray(offset).toString('utf8');
  return tail.split(/^(?=## )/m).filter(s => s.trim());
}
