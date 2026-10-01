# Campaign refresh fixture

This App covers the timed content and offline cache part of V-PKG-09. The Phase 1
plan's `live-app-content` section requires refresh through approved backend
network access, opening-time and bounded in-use refresh, notification integration
refresh, draft preservation, and explicit cache expiry. It does not prescribe a
new cache API. Existing `chariox.http.request`, `chariox.state.transaction`,
incoming App events and the managed view bridge supply the required runtime
operations. Cache durations and campaign selection are App policy.

The backend fetches one closed JSON record from the signed source
`https://campaigns.example.com/current.json`. This reserved example destination
is a fixture placeholder, not a deployed service. To install a working fixture,
the publisher must replace the source and the matching manifest origin before
packing a signed local release. No credentials or live publisher keys are needed
for these tests.

```json
{
  "id": "autumn",
  "title": "Autumn campaign",
  "body": "Early access opens today",
  "startsAtMs": 1800000000000,
  "endsAtMs": 1800007200000
}
```

The first read attempts a refresh. Views poll the same tool every five seconds
while visible; agent calls and configured `campaign_changed` inbox deliveries
use the same handler. A persisted compare-and-set claim limits HTTP attempts to
one per minute, including failures and restarts. HTTP has a five-second deadline.
Concurrent callers share the current refresh. No recurring worker timer or wake
keeps the App alive after all work ends.

The cached record is fresh for five minutes and usable offline for one hour,
both capped by the campaign's end time. Before its start time the view shows
`scheduled` without campaign text. After the end time or offline expiry the
backend returns no campaign text. Failed or malformed responses retain the last
good record without extending its expiry. Cache metadata includes fetch time,
fresh expiry, offline expiry, next refresh time and failure status.

The source supplies plain text, not HTML, URLs or executable templates. Unknown
fields, invalid times, long strings and responses over 16 KiB are rejected. The
SDK's buffered HTTP limit is 512 KiB before the App's smaller validation limit.
The view uses `textContent`; packaged JavaScript remains the only view code.
It updates campaign nodes without replacing the draft control, its selection or
focus. The draft is stored separately in view-origin session storage. A bridge
failure clears campaign text because the view cannot verify its expiry.

Run on the Phase 1 Linux builder:

```sh
p1b-run.sh "$WT" node --test examples/apps/campaign/test/campaign.test.mjs
p1b-run.sh "$WT" cargo test -p chariox-app-package --test campaign_fixture
```

The Node tests run the real SDK, peer and framed stream transports with test
state and HTTP brokers. State is written under an owned temporary directory and
reloaded when the test worker restarts. View tests use small DOM stand-ins. The
Rust test packs and verifies the actual bundle using an in-memory deterministic
test identity. These checks do not claim a native sandbox, production HTTP broker
or managed Chromium live drill. The existing joint-release, API mismatch and
stale bridge portions of V-PKG-09 still need the lead's live validation.
