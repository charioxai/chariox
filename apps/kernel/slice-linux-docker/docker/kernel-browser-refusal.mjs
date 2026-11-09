// Host-only policy errors; page exceptions and arbitrary error codes never qualify.
const reasons = new Set(['not_focused_agent','foreign_owner','stale_epoch','stale_reference','not_granted','sensitive_requires_focus']);
export class UserDomainRefusal extends Error {
  constructor(reason, race) {
    if (!reasons.has(reason)) throw new TypeError('Unknown user-domain refusal reason');
    super('User-domain request refused');
    this.code = 'user_domain_'+reason;
    // Diagnostics only: a refusal the viewer answers by rebinding (a retired region, a display it just suspended).
    if (race !== undefined) this.race = race;
  }
}
// Region capture failures the person can act on. Not policy refusals: the code and text reach the viewer as written.
const captureMessages = {
  view_resized: 'The page view changed size; select the region again',
};
export class CaptureRefusal extends Error {
  constructor(reason) {
    super(captureMessages[reason]);
    this.code = 'capture_' + reason;
  }
}
// MP-08/MP-10/MP-11: a fixed, pre-dispatch coordinate refusal an agent can act on.
export class PointerOutsideViewport extends Error {
  constructor() { super('MD-2: pointer outside viewport; x/y are CSS pixels inside state viewport css_width x css_height (screenshot pixels / device_scale_factor)'); }
}
// MP-08/MP-10/MP-11: fixed texts for the agent-only reads (region screenshot, scroll to a node, save, wait). Each names
// what the agent changes next and never carries a page value, URL or path.
const toolReasons = new Map([
  ['region_conflict', 'screenshot takes one of node_ref, clip or full_page, not several'],
  ['node_ref_invalid', 'node_ref must be a top-level node_ref from a fresh snapshot (backend:N)'],
  ['node_unavailable', 'the node is gone or has no box on the page; take a fresh snapshot'],
  ['node_not_visible', 'the node has no visible area in the viewport'],
  ['clip_invalid', 'clip needs integers x, y >= 0 and width, height >= 1 inside the state viewport (css_width x css_height)'],
  ['image_too_large', 'the image is too large to deliver; use a clip or node_ref'],
]);
export class AgentToolRefusal extends Error {
  constructor(reason) {
    const text = toolReasons.get(reason);
    if (!text) throw new TypeError('Unknown agent tool refusal');
    super(text);
    this.code = 'kernel_browser_failed';
    this.reason = reason;
  }
}
