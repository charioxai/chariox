# Isolated worker source mounts (MP-08 / MP-10 / MP-11)

Container drill launchers should pass `readOnlyWorktreeMounts(worktree)` from
`read-only-worktree-mounts.mjs` into their Docker argument array. It mounts the
selected source read-only and, for a linked worktree, its referenced Git common
directory at the same absolute path read-only. Do not replace these mounts with
the checkout's parent directory. Worker state, writable task repositories and
product-materialized provider profiles remain separate lane-owned mounts.

The optional `CHARIOX_WORKER_GIT_TEST_IMAGE` runs the real Git check inside that
existing tools image. Set disk-backed `TMPDIR` before running the tests. These
checks prove source/Git mount behavior; real leased-provider and client drills
remain separate MP-10 acceptance requirements.
