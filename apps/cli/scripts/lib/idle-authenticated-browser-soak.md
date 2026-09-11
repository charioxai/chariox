# Idle authenticated browser soak

This runner owns the CHA-16 24-hour idle browser gate. It uses only a loopback HTTP fixture, a random synthetic session cookie, and a disposable Chromium profile. It never contacts Google or another external identity provider and never accepts credential input.

Run it only as the non-root owner of a disposable non-development slice. Do not restart a kernel or service to make preflight pass. Evidence is written outside the repository with mode `0600`; raw cookie and profile-marker values are held only in memory or the temporary profile and are scanned out of retained JSON evidence.

Before a full launch, inspect the prior active-soak evidence and run:

```sh
node apps/cli/scripts/live-idle-authenticated-browser-soak.mjs --preflight \
  --max-cpu-percent 300 --max-rss-mb 2048 --max-processes 32 --min-free-disk-mb 1024
node apps/cli/scripts/live-idle-authenticated-browser-soak.mjs --smoke
```

The full detached command is:

```sh
node apps/cli/scripts/live-idle-authenticated-browser-soak.mjs --detach \
  --duration-seconds 86400 --health-interval-seconds 30 --sample-interval-seconds 30 \
  --max-cpu-percent 300 --max-rss-mb 2048 --max-processes 32 --min-free-disk-mb 1024
```

The runner refuses root Chromium, occupied CDP ports, insufficient initial resources, repository-local evidence, and another live owned run under the evidence root. Every health checkpoint must advance controller, authenticated-session, health, and persistent-profile-marker counters. Every resource sample fail-closes on the declared limits. Signal traps terminate only exact recorded process groups and retain a cleanup ledger, failure marker, status, samples, source SHA, and available image release-manifest identity.

At each 30-minute operator checkpoint inspect `runner.pid`, `status.json`, `resource-samples.jsonl`, `failure.json`, and `cleanup-ledger.json`. Confirm the PID is live; elapsed time and all four counters increased; Chromium and Browser Controller remain owned and live; RSS, CPU, process count, and disk remain within limits; no failure marker exists; and `cleanupReady` remains true.

A run proves the exact source and image recorded in its preflight/result only. If the image source commit predates the protocol-321 browser-import work, the run is useful stability evidence but is not final-head acceptance.
