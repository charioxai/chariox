// MP-11 owner amendment, 2026-10-05. Whole-file anchors bind the complete blob,
// including helpers with no managed-selector matches. A directory rule covers
// future siblings; content rules catch boundary operations outside those roots.
export const SECURITY_SCOPE_SCHEMA = 'chariox.mp11.security-scope.v1';
export const SECURITY_CLASSES = Object.freeze({
  credentials: {
    purpose: 'Vault, credential and provider-account material handling',
    path: /(?:^|\/)(?:secret(?:_redaction)?|vault|credentials?|provider[_-]accounts?|account_profile|managed_context|account_credential|account_handoff|credential_environment)(?:[\/._-]|$)/i,
    content: /(?:\.credentials\.json|CLAUDE_CONFIG_DIR|CODEX_HOME|OPENCODE_CONFIG_DIR|Keychain|keychain|vault[_-](?:key|secret)|SecretString)/,
  },
  relay: {
    purpose: 'Relay admission, scoped tokens, peer identity and protocol gating',
    path: /(?:^apps\/relay\/|relay[_/-](?:auth|crypto|peer|client|identity|token)|(?:^|\/)relay\.(?:rs|ts)|(?:^|\/)cloud-relay\.)/i,
    content: /(?:verify_relay|RelayTokenClaims|RelayPeer|relay_peer_protocol|RELAY_PEER_PROTOCOL|relay.*(?:admission|token_scope)|validate.*relay)/i,
  },
  signing: {
    purpose: 'Release/image signing, trust pins, cosign, Rekor and receipt verification',
    path: /(?:managed_bootstrap\/|publisher_trust|(?:^|\/)(?:sign[-_]|.*(?:release|image)[-_](?:verify|verification)|.*verify[-_](?:release|image))|(?:^scripts\/.*release)|(?:^deploy\/managed-kernel\/))/i,
    content: /(?:cosign|[Rr]ekor|verify_signature|verify_strict|Signature::from|VerifyingKey|release.public.key|trust.pin|verify.*receipt)/,
  },
  sandbox: {
    purpose: 'Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup',
    path: /(?:sandbox|\.entitlements$|managed_isolation|seccomp|apparmor|cgroup|protected[-_]|slice-linux-docker\/(?:provision|slice-data-volume)|worker_process\/platform_|storage_(?:linux|macos)\/)/i,
    content: /(?:\bbwrap\b|\bunshare\b|\bsetns\b|CLONE_NEW|PR_SET_NO_NEW_PRIVS|SECCOMP|cgroup\.procs|--security-opt|--cap-drop|--privileged)/,
  },
  apps: {
    purpose: 'App capability admission, publisher keys and host-action offers',
    path: /(?:app-package\/src\/|app[_/-](?:publisher|install|package_preparation|catalog|capability|host_action)|host[_-]action|app-dev-loop|publisher_trust)/i,
    content: /(?:HostActionOffer|host_action_offer|admit_capability|CapabilityAdmission|publisher.*(?:private_key|signing_key))/,
  },
  access: {
    purpose: 'Kernel access, sudo and passkey gates',
    path: /(?:kernel[_-]access|passkey|(?:^|\/)sudo[-_.])/i,
    content: /(?:KernelAccess|Passkey|passkey|sudoers|NOPASSWD|critical_approval)/,
  },
  signals: {
    purpose: 'Process signal and kill guards, including drill cleanup',
    path: /(?:process[_-]spawn|process[_-]group|runtime[_-]signals|launch_process|signal[_-]guard)/i,
    content: /(?:\.kill\s*\(|\b(?:kill|killpg|pkill|killall)\b|libc::(?:kill|raise)|SIG(?:TERM|KILL|INT)|TerminateProcess)/,
  },
  browser: {
    purpose: 'Protected frames, Vault masking and isolated-world observation capture',
    path: /(?:browser-controller|browser_controller|browser[_-](?:snapshot|observation)|protected[_-]frame|vault[_-]mask)/i,
    content: /(?:Page\.createIsolatedWorld|protectedFrames?|protected_frames?|vaultMask|vault_mask|isolatedWorld|isolated_world)/,
  },
});

// Explicit trust roots: a newly added/unmapped anchor class inside these roots
// cannot become ordinary source. Unknown formats are rejected by the inventory.
export const SECURITY_ROOTS = Object.freeze([
  'apps/relay/', 'apps/kernel/src/secret/', 'apps/kernel/src/managed_bootstrap/',
  'apps/kernel/src/provider/managed_isolation/', 'apps/app-worker/src/',
  'apps/app-storage-helper/', 'packages/app-package/src/',
  'packages/app-runtime/src/publisher_trust/',
  'packages/app-runtime/src/worker_process/', 'packages/app-runtime/src/worker_peer/',
  'packages/app-runtime/src/storage_linux/', 'packages/app-runtime/src/storage_macos/',
  'deploy/managed-kernel/',
]);

export function classifySecurityPath(path, text = '') {
  if (/(?:^|\/)(?:tests?|testdata|fixtures?|__tests__|snapshots?)(?:\/|$)|(?:[\/._-](?:test|tests|spec|bun-test))\.[^/]+$/i.test(path)) return SECURITY_CLASSES.signals.content.test(text) ? ["signals"] : [];
  const classes = Object.entries(SECURITY_CLASSES)
    .filter(([, rule]) => rule.path.test(path) || rule.content.test(text))
    .map(([name]) => name);
  if (!classes.length && SECURITY_ROOTS.some(root => path.startsWith(root))) return ['unknown'];
  return classes;
}

export const B211_PARITY_ITEMS = Object.freeze(['B211-KEY', 'B211-APP', 'B211-CAPTURE']);
export function evaluateParityMatrix(matrix, source) {
  const reasons = [];
  if (matrix?.schema !== 'chariox.mp11.behavioural-parity.v1'
    || matrix?.sourceCommit !== source.commit || matrix?.sourceTree !== source.tree
    || !Array.isArray(matrix?.rows)) return { status: 'pending', reasons: ['MP-11 exact-source behavioural parity matrix missing or stale'] };
  const required = [...Array.from({ length: 10 }, (_, i) => `MP-${String(i + 1).padStart(2, '0')}`), ...B211_PARITY_ITEMS];
  for (const id of required) {
    const rows = matrix.rows.filter(row => row.id === id);
    if (rows.length !== 1 || !rows[0].evidence?.trim()
      || !(rows[0].status === 'GREEN' || (rows[0].status === 'RED'
        && rows[0].ownerVisibleDisposition?.trim()))) reasons.push(`${id}: evidence-backed GREEN or owner-visible RED disposition required`);
  }
  // Additional parity items cannot be hidden by an allowlist.
  for (const row of matrix.rows) {
    if (!row.id || !row.evidence?.trim() || !(row.status === 'GREEN'
      || (row.status === 'RED' && row.ownerVisibleDisposition?.trim()))) reasons.push(`${row.id ?? 'unknown'}: unresolved parity row`);
  }
  return { status: reasons.length ? 'fail' : 'pass', reasons };
}

export function evaluateSecurityGate({ entries, gaps = [], staleReviews = 0, parity }) {
  const unknown = entries.filter(e => !Object.hasOwn(SECURITY_CLASSES, e.securityClass)).length;
  const unreviewed = entries.filter(e => e.semanticDisposition.status !== 'reviewed').length;
  const findings = entries.filter(e => e.semanticDisposition.disposition === 'removal_required').length;
  const summary = { anchors: entries.length, unknown, unreviewed, findings, staleReviews, gaps: gaps.length };
  return { status: entries.length > 0 && !unknown && !unreviewed && !findings
    && !staleReviews && !gaps.length && parity?.status === 'pass' ? 'pass' : 'fail', summary, parity };
}
