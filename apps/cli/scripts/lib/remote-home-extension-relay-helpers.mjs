import { createHmac } from "node:crypto"

function base64url(input) {
  return Buffer.from(input).toString("base64url")
}

export function createRemoteHomeExtensionRelayTokenFactory({
  issuer,
  secret,
  realm,
  accountId = "remote-home-extension-drill-account",
}) {
  function signRelayToken(claims) {
    const payload = base64url(JSON.stringify(claims))
    const signature = createHmac("sha256", secret).update(payload).digest("base64url")
    return `arroba-scoped-v1.${payload}.${signature}`
  }

  function relayClaims({ subject, subjectKind, actions, userId = null }) {
    return {
      issuer,
      subject,
      subject_kind: subjectKind,
      realm_id: realm,
      allowed_actions: actions,
      allowed_targets: null,
      issued_at_ms: Date.now(),
      expires_at_ms: Date.now() + 10 * 60_000,
      token_id: `${subject}-${Date.now()}`,
      account_id: accountId,
      organization_id: null,
      user_id: userId,
      device_id: subject,
      machine_id: subjectKind === "kernel" || subjectKind === "machine" ? subject : null,
      client_id: subjectKind === "client" ? subject : null,
      public_key_thumbprint: `${subject}-thumbprint`,
      entitlements_version: "drill",
    }
  }

  function daemonToken(subject, userId) {
    return signRelayToken(relayClaims({
      subject,
      subjectKind: "kernel",
      actions: [
        "daemon_register",
        "daemon_heartbeat",
        "client_metadata_read",
        "client_connect",
        "packet_route",
        "peer_request",
        "peer_event",
      ],
      userId,
    }))
  }

  function clientToken(userId) {
    return signRelayToken(relayClaims({
      subject: `client-${userId}-${process.pid}-${Date.now()}`,
      subjectKind: "client",
      actions: ["client_connect", "client_metadata_read", "packet_route"],
      userId,
    }))
  }

  return { daemonToken, clientToken }
}
