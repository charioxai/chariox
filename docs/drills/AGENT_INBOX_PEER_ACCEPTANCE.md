# Peer authority and pending inbox batches

MP-08 / MP-09 / MP-10 / MP-11: `scripts/drills/inbox-peer-acceptance.mjs`
checks AO-PE-03 and AO-IN-02 through private real kernels, an encrypted relay,
official provider CLIs, and optionally the shipped TUI in a real PTY.

Build the kernel, relay, kernel client and CLI first. Pass your own binaries and
keep both state and evidence outside the repository:

```sh
node scripts/drills/inbox-peer-acceptance.mjs \
  --kernel-binary /path/to/chariox-kernel \
  --relay-binary /path/to/chariox-relay \
  --state-root /root/.chariox/dev/inbox-peer-drill \
  --evidence-dir /path/outside/the/repository/evidence \
  --provider codex --codex-profile-path /path/to/existing/native/codex-profile \
  --scenario inbox --placement local --tui yes
```

Run `--scenario inbox` for both `--provider codex` (gpt-6-luna, low) and
`--provider claude` (Haiku 5.5, low), with `--placement local` and `other`.
Run `--scenario peer` at both placements. `--tui yes` submits prompts through the
built TUI prompt handler and captures terminal output plus automation snapshots.
Without it the same prompts enter through local IPC.

Codex profiles are linked through the product's native-directory registration.
Claude imports the explicitly authorized `CLAUDE_TOKEN_FILE` into A's Vault
(default `/root/.claude-lanes/oauth-token.chariox`). B receives its independent
profile through `CopyProviderAccountToKernel`, with official verification over
the encrypted relay. No provider credential files are copied by the driver.
The owner enrollment/directory endpoints use the existing loopback fixture;
this does not validate hosted Cloud enrollment or provider-account-copy/PAC-01.

The peer scenario requires a real child to invoke the forbidden room-peer spawn,
then the room agent to create a peer and receive its automatically followed turn
end. A Required parent must fail to create a Yolo peer and succeed at creating a
Required peer. Agent state independently confirms the resulting authority.

The inbox scenario uses three followed room workers in another Room. Their
nonurgent completions must all be durably pending while the orchestrator holds
its initial turn. Only then does the driver release that turn. It requires one
wake prompt shared by all three completions, a model inbox read containing all
three, and handled acknowledgements. It then unfollows the workers and sends
one reply-requested message: the recipient's durable wake must contain only that
request, and its real final answer must reach the sender.

This distinguishes batching from provider completion timing. As specified in
`docs/design/sub-agents.md` section 3.4, batching covers what is pending when the
wake is built, without a timer or coalescing window. Automatic `child_finished`
events are urgent and may steer a running parent; independently finishing child
providers may therefore produce separate wakes. The pending peer batch tests
coalescing without altering that urgent behavior.

`result.json`, public history, inbox/task records, account-copy receipts and TUI
captures are retained as evidence. Private kernels, relay, provider descendants,
TUI and state are cleaned up on success or failure.
