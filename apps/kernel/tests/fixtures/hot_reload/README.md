# Previous-release handover documents

`handover-<release>.json` is the document the kernel of release `<release>` wrote on a hot reload
(SIGUSR2) with a Claude turn and a Codex turn running, captured as the new program received it.
The unit tests in `apps/kernel/src/hot_reload/read.rs` read every file here with this build, so a
handover shape that gained a required field without a serde default (or changed without a
`HANDOVER_FORMAT` bump) fails the build.

At each release: capture the release's own document, add it, and delete the older one (a build
reads its own handover format and the previous release's, nothing older).

Capture (b3; `PREVIOUS` is the release's kernel binary, `CANDIDATE` the build under test):

    CHARIOX_HOT_RELOAD_KERNEL=$PREVIOUS CHARIOX_HOT_RELOAD_UPGRADE_TO=$CANDIDATE \
    CHARIOX_HOT_RELOAD_CAPTURE=$PWD/handover-<release>.json \
    cargo test -p chariox-kernel --test hot_reload a_codex_turn_continues_beside_a_running_claude_turn

The document holds ids, run tokens and paths of a throwaway test kernel only; no credential.
