# Idle authenticated browser soak

This runner owns the CHA-16 24-hour idle browser gate. It uses only a loopback HTTP fixture, a random synthetic session cookie, and a disposable Chromium profile. It never contacts Google or another external identity provider and never accepts credential input.

Run it only as the non-root owner of a disposable non-development slice. Do not restart a kernel or service to make preflight pass. Authoritative evidence defaults to `~/.codex/evidence/browser-computer-use/idle-authenticated-browser-soak/<run-id>/` with mode `0600`; development-state directories may contain runtime scratch only. Raw cookie and profile-marker values are held only in memory or the temporary profile and are scanned out of all terminal JSON, process logs, and `runner.log` after terminal files exist.

Before a full launch, inspect the prior active-soak evidence and run:

```sh
node apps/cli/scripts/live-idle-authenticated-browser-soak.mjs --preflight \
  --max-cpu-percent 300 --max-rss-mb 2048 --max-processes 32 --min-free-disk-mb 1024
node apps/cli/scripts/live-idle-authenticated-browser-soak.mjs --smoke \
  --max-cpu-percent 300 --max-rss-mb 2048 --max-processes 32 --min-free-disk-mb 1024
```

The full detached command is:

```sh
node apps/cli/scripts/live-idle-authenticated-browser-soak.mjs --detach \
  --duration-seconds 86400 --health-interval-seconds 30 --sample-interval-seconds 30 \
  --max-cpu-percent 300 --max-rss-mb 2048 --max-processes 32 --min-free-disk-mb 1024 \
  --evidence-root "$HOME/.codex/evidence/browser-computer-use/idle-authenticated-browser-soak"
```

The runner refuses root Chromium, occupied CDP ports, insufficient or non-finite initial resources, a non-empty dedicated run directory, repository-local evidence, and another live owned run under the evidence root. A detached launch also requires a passed smoke from the same clean source and passes the child the exact preflight/smoke contract. Every health checkpoint requires the controller's ready state and exact PID, observes a fresh cookie-authenticated browser request without navigating, and advances controller, authenticated-session, health, persistent-profile-marker, and fresh-request counters. Chromium is restarted once with the same profile; the post-restart checkpoint must observe both the cookie and Chromium local-storage marker. Every resource sample fail-closes on the declared limits. Signal traps terminate exact PID/start-time identities and their owned process-group descendants, then prove there are no owned survivors or listeners.

At each 30-minute operator checkpoint inspect `runner.pid`, `status.json`, `resource-samples.jsonl`, and `failure.json`. Confirm the PID is live; monotonic elapsed time and all five counters increased; the latest checkpoint records the exact ready controller PID and a live browser; RSS, CPU, process count, and disk remain finite and within limits; and no failure marker exists. `cleanupReady` intentionally remains false while the run owns processes. A pass requires cadence-minimum coverage and one additional final fresh authenticated/browser/controller checkpoint, terminal evidence scanning, and `cleanup-ledger.json` proving cleanup before `status.json` becomes passed.

A run proves the exact source and signed-image identity recorded in its preflight/result only. The loopback fixture proves idle cookie/profile/controller durability; it does not prove Google authentication or complete managed-machine recreation.
