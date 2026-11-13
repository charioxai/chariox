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
  ['payment_approval_required', 'This payment needs a live owner approval. Fill the saved card again to ask.'],
  ['payment_already_sent', 'This payment was already sent and may have completed. Inspect the receipt before starting another payment.'],
  ['payment_target_unavailable', 'The checkout action could not be checked. Observe it again.'],
  ['region_conflict', 'screenshot takes one of node_ref, clip or full_page, not several'],
  ['node_ref_invalid', 'node_ref must be a top-level node_ref from a fresh snapshot (backend:N)'],
  ['node_unavailable', 'the node is gone or has no box on the page; take a fresh snapshot'],
  ['node_off_screen', 'the node is outside the viewport; scroll to it first (input scroll with target set to this node_ref), then take the screenshot'],
  ['clip_invalid', 'clip needs integers x, y >= 0 and width, height >= 1 inside the state viewport (css_width x css_height)'],
  ['scroll_target_conflict', 'scroll takes a target node_ref or x, y and deltas, not both'],
  ['wait_condition_invalid', 'wait takes one of text, node_ref or url_contains, and a timeout_ms from 0 to 30000'],
  ['image_too_large', 'the image is too large to deliver; use a clip or node_ref'],
  ['save_source_conflict', 'save takes a node_ref or a url, not both'],
  ['save_path_invalid', 'save needs a file path inside the Room workspace that is not a directory'],
  ['workspace_unavailable', 'the Room workspace is unavailable'],
  ['save_url_unsupported', 'save needs an http(s) or data URL, or the node_ref of an image, link, media or canvas element'],
  ['save_fetch_failed', 'the page session could not fetch the resource'],
  ['save_too_large', 'the resource is larger than 25 MiB'],
  ['save_canvas_unavailable', 'the canvas cannot be read (cross-origin content) or the page holds a protected field'],
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
