# MP-08 / MP-10 / MP-11 — Kernel Access acceptance matrix

Scope: Kernel Access plan §7 and §9.2 PRs 3, 5, 7, 8, 10, decisions D1–D16.
Linux source identity is `74e50b787` + PR #872 = `97d7d9e20`, local 416,
relay 70. `/meta` retirement (#873, local 430) is held outside this campaign.
The 2026-10-05 MP-11 narrowing applies: this is KA/passkey and signal-guard
security evidence, not a demand for exact-blob review of unrelated source.

Run `python3 scripts/kernel-access-validation.py --output <new external dir>`
on builder2. It serializes Rust through the shared compile lock, uses four
jobs, private synthetic state and existing cfg(test) vault/passkey fixtures,
records commands/source/binary hashes/resources/cleanup, and stops its own
processes if MemAvailable falls below 16 GiB or disk free below 10 GiB.
It never uses an owner vault, real provider profile or remote service.
Use a clean committed checkout: staged, unstaged and untracked source changes
are refused. Only top-level `LANE_STATUS.md` and `PUSH_READY.md` are excluded.
The runner rechecks HEAD/tree and cleanliness before and after stages and
before success. Kit-only reuse requires a matching HEAD/tree, binary hash and
`source_policy: clean_checkout` receipt; older receipts require a rebuild.
SIGTERM/SIGINT trigger birth-checked pidfd teardown, private-state removal and
a cancellation receipt. Runner regression fixtures test these controls, not
kernel or human acceptance: `python3 scripts/kernel-access-validation-regression.test.py`.

Evidence classes: **WIRE** = real kernel TCP/Unix/MCP listeners and clients;
**STATE** = actual kernel services/router with synthetic runs and explicit
lifecycle controls; **OS** = actual Linux process/PTY launches; **CLIENT** =
first-party client tests with private socket fixtures; **HUMAN** = official
provider, rendered terminal/passkey/UI on the live Mac stack. Clock expiry and
PID reuse are injected deterministically; no actual PID wraparound is claimed.

All automated rows below must pass; then run `SUDO_KIT.md`. WIRE/STATE/OS
success alone does not mean “sudo validated”, fresh Path-1 parity, or MP closure.
The final evidence report is authoritative for executed results and gaps.

| ID / PR | Required observation | Automated selector (existing tests) / class | Mac gate |
| --- | --- | --- | --- |
| KA-03.1 / 3 | Owner terminal attached and waiting-room terminal get identical popup; one answer clears both; late answer refused | `one_passkey_prompt_reaches_every_terminal_and_the_first_answer_closes_it` — WIRE | H1: actual local TUI, web, remote TUI; unattached owner included |
| KA-03.2 / 3 | Concurrent correct answers resolve once; refusal needs no passkey | `two_terminals_answering_at_once_resolve_the_prompt_once`, `the_first_answer_closes_the_prompt_and_a_later_one_is_already_answered` — STATE | H1/H2: one winning terminal and one audit |
| KA-03.3 / 3 | Wrong passkey leaves popup open and counts toward common lockout; expiry returns error | `wrong_passkeys_keep_the_prompt_open_and_count_toward_the_lockout`, `an_unanswered_prompt_expires_with_its_decision` — STATE; grant timeout — WIRE | H1: one wrong attempt; avoid deliberate live lockout |
| KA-03.4 / 3 | Only shared-session host gets/answers popup; host credential cannot submit passkey | `a_critical_approval_is_a_passkey_prompt_for_its_owner_alone`, class passkey tests — STATE; popup host exclusion — WIRE | H2: actual guest and host terminals |
| KA-03.5 / 3 | No passkey in popup/event/audit/transcript; paste preserved or refused | `no_passkey_reaches_a_log_an_audit_or_a_popup`, client popup tests — STATE/CLIENT | H1: owner checks private UI; evidence excludes input values |
| KA-05.1 / 5 | Popup identifies OS executable/PID, session and selected term; helper may name verified ancestor | `kernel_access_grants_identify_holder_descendants_and_refuse_sibling_scope_and_critical` — WIRE | H3: persistent holder and real product CLI descendant |
| KA-05.2 / 5 | Descendant allowed; sibling, forged holder and reused birth identity refused | same selector; `kernel_access_grants_expire_revoke_on_pid_reuse_and_real_rotation`, `peer_identity_is_os_verified_and_reused_pid_or_version_fails` — WIRE/OS (reuse injected) | H3: real sibling refusal; no forced OS PID reuse |
| KA-05.3 / 5 | Scope deny by default, list filtered, global/cross-session denied; scope catalog exhaustive | grant selector; `kernel_access` policy/catalog tests — WIRE/STATE | H3: holder only sees its session |
| KA-05.4 / 5 | Explicit revoke/expiry immediately closes idle subscription and refuses reconnect | grant identify, expiry and `kernel_access_overlapping_grants_keep_subscriptions_bound_and_select_by_session` — WIRE | H4: real expiry, revoke and idle observation |
| KA-05.5 / 5 | Holder exit/session end cut off descendants/subscribers | `kernel_access_grants_process_exit_session_end_and_no_terminal_timeout` — WIRE | H4: quit holder, close throwaway session |
| KA-05.6 / 5 | Extension popup requires new passkey and starts new selected term | grant identify selector, notice advanced through existing fixture — WIRE | H4: actual clock and rendered extension popup |
| KA-05.7 / 5 | Rotation revokes grants and pending requests | grant expiry/rotation selector — WIRE (real synthetic vault rotation) | H9: owner-operated rotation |
| KA-05.8 / 5 | Grant cannot answer critical or credential prompts/mint authority; kernel-spawned agents inherit nothing | grant identify, `kernel_access_external_holder_cannot_manage_vault_or_choose_new_passkey`, `kernel_access_external_holder_cannot_answer_vault_unlock` — WIRE | H3/H6: critical refused; spawned ordinary agent probe refused |
| KA-07.1 / 7 | Missing/wrong-token TCP upgrade returns 401, safe token-file/socket guidance; right token works | `laptop_kernel_websocket_enforces_local_tokens` — WIRE | H0/H3: real first-party terminal and raw tokenless/wrong upgrade |
| KA-07.2 / 7 | Connection class controls terminal authority; existing host-token path works; Origin refused | class/kernel-access tests — STATE; `kernel_websocket_auth_rejects_missing_or_wrong_tokens_before_accepting_requests`, Origin test — WIRE | Human terminals/host topology acknowledged; relay E2E H1 |
| KA-08.1 / 8 | Fresh passkey each sudo; starts separate normal turn, preserves ordinary tools | `sudo_fresh_popup_starts_one_separate_turn_through_normal_admission`, `sudo_runtime_mcp_uses_shared_router_and_removes_tool_at_yield` — STATE + real HTTP MCP | H5: official provider edits a harmless file and uses sudo request tool |
| KA-08.2 / 8 | Protocol 460 (A04): finite window bound to owner work; survives waits only for its correlated continuations; expiry, work end, revoke and restart end it; no inheritance | `sudo_without_durable_tasks_ends_with_its_turn`, `sudo_window_survives_waits_and_admits_only_its_own_work`, `sudo_window_ends_when_its_owner_work_ends`, `sudo_popup_defaults_to_one_hour_and_refuses_more_than_eight` — STATE | H5: real window expiry, extension and restart drill |
| KA-08.3 / 8 | Interrupt/replay cannot restore authority | `sudo_interrupt_and_replayed_prompt_cannot_restore_elevation`, shell interrupt test — STATE/OS | H7: real UI interruption |
| KA-08.4 / 8 | Busy authorized sudo stays separate/memory-only; revoke-all before idle prevents dispatch | `sudo_busy_authorization_is_memory_only_and_revoke_all_prevents_dispatch`, `sudo_bound_turn_refuses_queued_steering_from_terminals_and_external_grants` — STATE | H7: busy official provider, authorize/revoke then release |
| KA-08.5 / 8 | Restart drops queued sudo and records notice, never restores prompt/authority | `sudo_restart_discards_queue_and_records_notice_without_prompt_content` — STATE (rebootstrap) | H8: coordinator-controlled restart of acceptance kernel |
| KA-08.6 / 8 | Rotation cancels queued and interrupts running sudo | `sudo_rotation_cancels_authorized_queue_before_idle_dispatch`, `sudo_rotation_interrupts_running_turn_and_revokes_grant` — STATE | H9: actual UI rotation |
| KA-08.7 / 8 | Protocol 460 (A04): agents never answer approvals, elevated or not; the owner answers once in a terminal | `sudo_agents_cannot_answer_any_approval`, `sudo_room_tools_preserve_host_authority_and_revocation` — STATE | H6: model attempt refused, owner answers normally |
| KA-08.8 / 8 | Spawned agent has no sudo; sudo cannot mint grants/sudo, read/export secret values, rotate or pair/invite | sudo exact-turn and `sudo_cannot_mint_authority_answer_sudo_popup_or_submit_passkeys`, `sudo_cannot_pair_invite_or_read_or_replace_relay_identity` — STATE | H6: spawned official provider refused, no real secret access attempt |
| KA-08.9 / 8 | External request needs live same-machine process grant; popup shows requester/target/session/full prompt; guest/TCP/foreign target refused | `external_sudo_unix_socket_requires_grant_projects_identity_and_expires_without_terminal` — WIRE; `external_sudo_popup_names_os_requester_target_session_and_full_prompt`, `external_sudo_only_granted_target_is_allowed_and_terminal_cannot_request` — STATE | H5: real `chariox sudo request` from granted holder descendant, approve then one turn |
| KA-08.10 / 8 | External revoke/exit/expiry cancels pending/queued request; final dispatch rechecks; no terminal expires clearly | external Unix selector; `external_sudo_revoked_holder_cancels_popup_and_busy_queue`, `external_sudo_source_cannot_reopen_a_session_ended_before_attach` — WIRE/STATE | H4/H7: revoke holder while sudo pending/busy |
| KA-10.1 / 10 | Linux/macOS launch uses dedicated OS session and existing isolation/scrub/Seatbelt path | `sudo_shell_piped_claude_has_dedicated_session_and_tracked_identity`, `provider_launch`, `provider_runtime::tests::launch`, `managed_isolation`, `pty::manager::tests` — OS Linux | H5: real Mac official provider; Linux does not establish Seatbelt runtime |
| KA-10.2 / 10 | Turn's shell CLI over Unix socket works, same command refused after actual completion | `sudo_shell_live_throwaway_kernel_cli_loses_authority_after_yield` with built `CHARIOX_SUDO_SHELL_CLI` — WIRE/OS | H5: official provider shell and next ordinary turn |
| KA-10.3 / 10 | Sibling joins target PGID but receives no authority; shared roots/reused PID refused | `sudo_shell_sibling_joining_process_group_has_no_authority`, `sudo_shell_process_identity_rejects_pid_reuse_and_shared_provider` — OS + identity injection | Mac counterpart; no membership-only authority |
| KA-10.4 / 10 | Old descendants and new children cannot inherit later sudo; interruption/rotation removes authority while process exists | `sudo_shell_excludes_old_descendants_and_their_new_children`, `sudo_shell_interrupt_and_rotation_drop_process_authority` — OS/STATE | H7/H9: next turn cannot use old authority |
| KA-R.1 | Authorization waits have no timeout/replay duplication; Unix protocol/client shapes stay valid | focused seven Node files, `local::api::tests::protocol_shapes` — CLIENT/STATE | H0: deployed/source protocol binding, no local-430 retirement build |

`kernel_access` and `sudo` selectors include subprocess entries marked ignored
only as child entry points. Their parent tests explicitly execute them with
`--ignored`; ordinary ignored counts do not waive a behavior assertion. A
runner must fail if a selector executes zero real tests. Lifecycle injections
and synthetic provider runs must always remain named in the final report.
