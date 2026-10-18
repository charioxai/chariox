// O1 (Miguel 2026-10-18, Browser displays only): a browser window capture on the owned desktop is
// redirected Manual, so the X server does not blit every Chrome swap onto the root, while nothing
// reads the root. The first root consumer (Computer Use, a desktop stream) latches the desktop to
// presenting and waits until every live window capture has switched; later captures start presenting.
export const newRoot = () => ({ requested: false, ready: null, followers: new Set() });
export function presentRoot(binding) {
  const root = binding?.root;
  if (!root) return Promise.resolve();
  root.requested = true;
  return root.ready ??= Promise.all([...root.followers].map(follow => follow())).then(() => {}, error => { root.ready = null; throw error; });
}
