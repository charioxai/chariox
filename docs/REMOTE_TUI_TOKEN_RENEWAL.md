# Hosted terminal authorization renewal

A hosted terminal relay token is a short-lived, key-bound transport grant.
`LocalIpcClient` renews Cloud terminal grants before expiry through the attached
kernel's existing `IssueCloudRelayClientToken` request. The kernel calls Cloud's
`/relay/token` endpoint using its private Cloud authority. The terminal never
receives that authority. Cloud remains the control plane; encrypted terminal
requests and events continue directly through the relay to the kernel.

Renewal retains the terminal subject, account, owner, realm, machine, session,
recipient key and connected target. The replacement cannot increase target
scope or reduce its action permissions. Both control and event sockets repeat
the existing `client_connect` exchange, acknowledge the same pinned kernel key,
and retain their subscriptions. A later reconnect uses the latest grant.
Local daemon protocol 472 versions this transport contract. The client probes
`terminal_relay_authorization_renewal_v1` in RelayStatus immediately after
connecting, and displays an explicit kernel upgrade message when absent.
Existing relay messages and unrelated Web/native feature minima are unchanged.

Transient issuance failures and target-offline relay closes reconnect and retry
within the current grant's lifetime. A refused renewal,
invalid replacement or expired grant ends terminal admission, stops retries,
and displays an authorization message directing the user to sign in or pair
again. The kernel session keeps running. Renewal never pairs a revoked terminal
again or changes the kernel's saved login client identity.

Opaque local/operator credentials are unchanged. Automatic Cloud renewal needs
a Cloud CLIENT token with account/user claims and the matching persistent
terminal key. Provider logins are independent of relay authorization.

## Validation

The focused SDK tests use short grants across multiple expiries, retain both
sockets and their encrypted event subscription, exercise transient issuance and target-loss recovery,
and stop on revocation or a different recipient key. Relay tests verify that
repeated `client_connect` retains the same identity and packet permissions.
The real-path drill also interrupts the owned kernel relay link during renewal
and checks a new client against a pre-change kernel for the early upgrade
message.

For an owned validation kernel and session, use the existing validation launcher
with `VALENV_EXPECT_TOKEN_EXPIRY=1`. This flag preserves the normal five-minute
pairing token instead of issuing an extended validation token:

```sh
VALENV_EXPECT_TOKEN_EXPIRY=1 node /w/valenv/scripts/relay-tui-smoke.mjs \
  /path/to/oss/apps/cli ws://127.0.0.1:OWN_PORT/kernel \
  /w/valenv/OWN_KERNEL_HOME OWN_SESSION_ID UNIQUE_LANE 1200 \
  /path/to/client-kit
```

Require zero disconnected checkpoints, connection-ended messages and shaper
failures, and verify that several fresh five-minute CLIENT grants were issued.
Revoke only the disposable Cloud terminal through `/clients/revoke`, confirm
renewal is denied and its terminal admission ends with the authorization
message. Do not change shared provider logins.
