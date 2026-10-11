// Coordinator callbacks operate real clients through existing admitted transports.
// This module creates no credentials, endpoints, grants or provider processes.
export const clients = ['web', 'local-tui', 'remote-tui'];
export const providers = ['codex', 'claude', 'opencode'];
export const codecs = ['videotoolbox', 'png'];
export const scenarios = ['retina-negative-origin', 'loss-reconnect', 'lock-sleep-wake', 'permission-revoke', 'helper-crash', 'signed-upgrade'];
export const matrixCases = Object.freeze(clients.flatMap(client => codecs.flatMap(codec => [
  ...providers.map(provider => Object.freeze({id:`${client}/${codec}/${provider}`, client, codec, provider, scenario:'real-app'})),
  ...scenarios.map(scenario => Object.freeze({id:`${client}/${codec}/${scenario}`, client, codec, scenario}))
])));
const hash = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const positive = value => Number.isFinite(value) && value > 0;
const refused = value => value === true;

function functional(proof, testCase, pins) {
  const route = testCase.client === 'local-tui' ? 'kernel' : 'hosted-wss';
  return proof?.live === true && proof.client === testCase.client && proof.route === route &&
    proof.helper_sha256 === pins.helper_sha256 && proof.kernel_sha256 === pins.kernel_sha256 &&
    proof.scenario === testCase.scenario && proof.provider === testCase.provider &&
    proof.codec === testCase.codec &&
    (testCase.codec === 'png' ? proof.backend === 'png' : ['videotoolbox-hardware','videotoolbox-software'].includes(proof.backend)) &&
    hash(proof.source_pixels_sha256) && proof.source_pixels_sha256 === proof.presented_pixels_sha256 &&
    refused(proof.protection_canary_absent) && refused(proof.independent_native_effect) &&
    refused(proof.stale_epoch_refused) && refused(proof.no_input_replay) &&
    refused(proof.scenario_verified) && refused(proof.owned_cleanup) &&
    typeof proof.evidence === 'string' && proof.evidence.startsWith('/Users/miguel/.codex/evidence/');
}
function performance(proof) {
  const m = proof?.metrics;
  // Shared display/report.py 30 Hz gate, RTT+50 ms input ack and DPR2 settle.
  return m && Number.isInteger(m.samples) && m.samples >= 30 &&
    [1,2].includes(m.dpr) && Number.isFinite(m.rtt_ms) && m.rtt_ms >= 0 &&
    positive(m.input_visual_p50_ms) && positive(m.input_visual_p95_ms) &&
    m.input_visual_p50_ms <= m.input_visual_p95_ms && m.input_visual_p95_ms <= m.rtt_ms + 50 &&
    positive(m.capture_publish_p95_ms) && m.capture_publish_p95_ms <= 50 &&
    positive(m.presented_fps) && m.presented_fps >= 30 &&
    Number.isFinite(m.settle_ms) && m.settle_ms >= 0 && m.settle_ms < (m.dpr === 2 ? 500 : 300) &&
    positive(m.egress_bps) && positive(m.budget_bps) && m.egress_bps <= m.budget_bps &&
    positive(m.peak_owned_rss_bytes) && m.peak_owned_rss_bytes <= 3 * 1024 ** 3;
}

async function bounded(callback, testCase, timeoutMs) {
  const controller = new AbortController();
  let timer;
  try {
    return await Promise.race([
      Promise.resolve().then(() => callback(testCase,{signal:controller.signal})),
      new Promise((_,reject) => { timer = setTimeout(() => { controller.abort(); reject(Error('matrix callback deadline')); },timeoutMs); })
    ]);
  } finally { clearTimeout(timer); }
}

export async function runMacClientMatrix({runCase, cleanup, pins, caseTimeoutMs = 60_000} = {}) {
  const report = {functional:'BLOCKED', performance:'BLOCKED', protocol:497, peer:106, rows:[]};
  if (typeof runCase !== 'function' || typeof cleanup !== 'function' ||
      !hash(pins?.helper_sha256) || !hash(pins?.kernel_sha256) ||
      !Number.isInteger(caseTimeoutMs) || caseTimeoutMs < 1 || caseTimeoutMs > 60_000) {
    report.rows = matrixCases.map(c => ({id:c.id,functional:'BLOCKED',performance:'BLOCKED',reason:'matching granted build and real coordinator callbacks required'}));
    return report;
  }
  for (const c of matrixCases) {
    const row = {id:c.id,functional:'FAIL',performance:'BLOCKED'};
    try {
      const proof = await bounded(runCase,c,caseTimeoutMs);
      if (proof?.blocked === true) row.functional = 'BLOCKED';
      else {
        row.functional = functional(proof,c,pins) ? 'PASS' : 'FAIL';
        row.performance = row.functional === 'PASS' ? (performance(proof) ? 'PASS' : 'FAIL') : 'BLOCKED';
        // Keep only the public receipt path and bounded metrics, never transport payloads.
        if (row.functional === 'PASS') row.evidence = proof.evidence;
      }
    } catch { row.reason = 'real case failed; preserve callback failure receipt'; }
    finally {
      try { if (await bounded(cleanup,c,5_000) !== true) { row.functional = 'FAIL'; row.performance = 'BLOCKED'; row.reason = 'owned cleanup unconfirmed'; } }
      catch { row.functional = 'FAIL'; row.performance = 'BLOCKED'; row.reason = 'owned cleanup failed'; }
    }
    report.rows.push(row);
    // A failure may leave an owned press. Never dispatch another mutation.
    if (row.functional === 'FAIL') {
      for (const remaining of matrixCases.slice(report.rows.length)) report.rows.push({id:remaining.id,functional:'BLOCKED',performance:'BLOCKED',reason:'earlier failure stopped mutation'});
      break;
    }
  }
  for (const field of ['functional','performance']) report[field] = report.rows.some(r=>r[field]==='FAIL') ? 'FAIL' : report.rows.every(r=>r[field]==='PASS') ? 'PASS' : 'BLOCKED';
  return report;
}
