# MP-03 / MP-07 / MP-11: signed service-policy transition

H and I's signed verifier requires `StateDirectory=chariox` and
`StateDirectoryMode=0700` in the worker artifact. Their installed upgrade
scripts verify the candidate with that frozen verifier. Removing those lines
cannot be a direct upgrade, even when updating a managed home, because the
verifier checks the packaged worker unit too.

Policy version 1 is the compatibility bridge. An absent version retains the
historical policy; an explicit `CHARIOX_PATH1_SERVICE_POLICY_VERSION=1` keeps
that same contract. K-A carries this policy and a supervisor that can read
legacy service-principal receipts as well as root-private receipts through its
privileged helper. Its upgrader understands both policies and journals access
metadata before changing live state.

Policy version 2 requires both roles to omit `StateDirectory` and
`StateDirectoryMode`. Its signed units set
`CHARIOX_PATH1_SERVICE_POLICY_VERSION=2`; the supervisor always uses the
root-private control-state helper. Unknown or duplicate versions fail closed.
K-B opts into version 2. It differs from K-A in service policy as well as the
package version. It is not a direct H/I upgrade candidate.

The publication and activation order is mandatory:

1. Review and sign K-A and K-B independently with the approved trust pins.
2. Offer K-A to H/I. Run their unchanged verifier and updater; retain legacy
   receipt access during this hop so their rollback tooling still works.
3. After K-A is active, offer K-B. K-A verifies version 2, publishes the
   ownership journal, stops services, and migrates control state to root.
4. On rejection before journaling, access stays unchanged. On activation failure
   or interrupted recovery, restore receipt contents and recorded access before
   restarting the previous supervisor. A target without helper capability gets
   legacy service-principal access, rather than unreadable private state.

Provisioning verifies the pinned manifest and supervisor artifact before querying
`--print-control-state-version`. A helper-capable pinned binary returns `1`;
H/I's unsupported-option exit `2` retains their original receipt permissions.
An unexpected result or mismatched pin fails before changing ownership.

`node --test scripts/path1-policy-transition.test.mjs` exercises the frozen
H/I policies. `scripts/managed-historical-policy-upgrade.test.mjs` additionally
accepts `MP07_SIGNED_H`, `MP07_SIGNED_I`, `MP07_RELEASE_PIN`, and
`MP07_BUILDER_PIN`: complete extracted signed trees and independent public pins.
It runs H/I's actual unchanged verifier/upgrader, then the versioned updater on
the same fake host. `MP07_CANDIDATE_SERVICE_POLICY=0` reproduces J's incompatible
worker artifact. These tests use synthetic target signatures, kernel health,
and host services. They do not establish signed K deployment, live providers,
Cloud publication order, real restart, or managed acceptance.
