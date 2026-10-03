# Legacy managed updater fixture

These files retain the exact managed updater and its local dependencies from
commit `8fa9246ea60b52a988da17a28652e61475ff52cd`. `provenance.json` records each
original repository path and SHA-256 of its unmodified bytes. The compatibility
test verifies those hashes before executing the historical updater. Do not update
these tools with the current implementation.

The fixture is source input, not rollout evidence. It makes the old-source
preactivation regression independent of Git history, shallow checkouts, squash
merges, and network access. The signed release/kernel fixtures are synthetic;
this test does not prove a real managed-machine rollout.
