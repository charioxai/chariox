# Runtime card rules

Only individual types in `docs/runtime-card-types.json` may be cards. Every type has exactly one of the three classes below. `docs/card-decisions.json` records Miguel’s approval for that exact type/class. An addition or class change without its matching decision fails lint and CI. Question is one type for both answer shapes.

The JSON is authoritative: the kernel includes it and client code is a checked projection. Cloud consumes the pinned OSS projection for its own cards as well as runtime cards. Unknown types are refused and logged at admission. Display changes never answer, cancel, resume or discard the underlying operation.

| Individual type | Class |
|---|---|
| Question | person action or answer |
| provider_native_permission | person action or answer |
| opencode_permission | person action or answer |
| codex_command_approval | person action or answer |
| codex_file_change_approval | person action or answer |
| claude_permission_hook | person action or answer |
| claude_workspace_trust | person action or answer |
| provider_account_login | person action or answer |
| provider_account_logout | person action or answer |
| provider_login_workflow | person action or answer |
| provider_auth_recovery | person action or answer |
| portable_claude_login | person action or answer |
| provider_account_copy | person action or answer |
| credential_enrollment | person action or answer |
| credential_secret_capture | person action or answer |
| credential_secret | person action or answer |
| vault_secret_request | person action or answer |
| vault_unlock | person action or answer |
| vault_passphrase_change | person action or answer |
| vault_payment_approval | person action or answer |
| sudo_task_approval | person action or answer |
| sudo_window_extension | person action or answer |
| card_action | person action or answer |
| owner_context | person action or answer |
| auth_md_claim | person action or answer |
| kernel_access_grant | person action or answer |
| kernel_enrollment | person action or answer |
| machine_acquisition | person action or answer |
| app_binding_permission | person action or answer |
| workspace_live_sync_permission | person action or answer |
| extension_registration_permission | person action or answer |
| skill_catalog_permission | person action or answer |
| terminal_permission | person action or answer |
| watched_process | person action or answer |
| app_install | person action or answer |
| app_update | person action or answer |
| app_publisher_enroll | person action or answer |
| app_validation | person action or answer |
| app_deployment_consent | person action or answer |
| app_clipboard_write | person action or answer |
| app_open_link | person action or answer |
| app_file_pick | person action or answer |
| app_file_export | person action or answer |
| app_view_action | person action or answer |
| app_view_confirmation | person action or answer |
| owner_handoff_click | live browser card |
| owner_handoff_input | live browser card |
| popup_browser_region | live browser card |
| notify_person_notice_origin_agent | asked-for notification |
| provider_sign_in_notice_origin_kernel | person action or answer |

## Authority and retirement

An approved type still needs its live requesting operation and existing responder authority. The preconditions, answerer checks, deadlines, supersession and cold restart rules remain in the owning domain. The following domain rules describe those lifetimes; membership in a domain does not authorize a new card type.

| Group | Trigger | Live state | Answerer | Retirement | Cold restart |
| --- | --- | --- | --- | --- | --- |
| Task | An owner blocker or task question is admitted | Current blocked revision or unanswered wake bound to its task | Eligible holder or Room owner | Answer, timeout, cancellation or supersession | Recheck the ledger and rearm current blockers |
| Native permission or input | Provider request, forwarded request or live popup | Live requester and matching provider turn or lease where applicable | Eligible holder or Room owner under native policy | Answer, timeout or end of the caller or turn | Requires a fresh request |
| Provider login | Login, logout, recovery, enrollment or account copy needs input | Current provider-account operation and requester | Authorized account or kernel owner | Answer, process exit, timeout or cancellation | Only a recovered operation can request input again |
| Vault, credential or payment | An exact secret, unlock, credential or payment operation needs input | Current requester and exact operation, with secret delivery scoped to its destination | Authorized owner | Answer, completion, timeout or abandonment | Requires a new live waiter |
| Runtime grant | An operation needs a missing permission | Current requesting operation and ungranted scope | Authorized owner or eligible holder | Answer, existing grant, timeout or abandonment | Requires a fresh request |
| App | An App install, publish, validation, deployment or host operation needs consent | Current pending operation and eligible App revision or caller | Authorized App owner or eligible holder | Answer, expiry, revocation or abandonment | Revalidate still-pending durable operations |
| Owner control | An owner decision, access request, hand-off or usage stop needs a response | Current operation, authorized owner, and applicable live requester or target | Authorized owner or eligible holder under the operation's policy | Answer, timeout, supersession or abandonment | Revalidate durable hand-offs and usage stops |

## Non-card events

Miguel 2026-11-28 ~09:15 Berlin, via the Mac (247894), supersedes the blanket pane-status rule. These display choices never change the underlying task/run state, pending answer channel, parent wake, message delivery or schedule firing.

| Existing event | Display |
|---|---|
| Provider stalled | Error in its agent pane; Resume/Start fresh and the parent wake remain |
| Agent time limit ended | Warning/status in its agent pane |
| Usage stop | Existing paused-until-reset display in its agent pane |
| Interrupted after restart | Error in its agent pane, identified by the durable interruption cause and revision |
| Subscription stopped | Error in its agent pane, from canonical subscription health |
| Workflow run update or failed run | That run in Workflows; no agent-pane row or card |
| Generic task blocked | No generated UI row or card; normal agent text and genuine Question remain |
| Message to an idle/retired agent | No generated UI row or card; delivery remains |
| Workflow could not start | No generated UI row or card; actual run/recovery state remains |
| Schedule update | No generated UI row or card; firing remains |
| Sub-agent update | No generated UI row or card; parent delivery remains |

A pending operation may have a pane-status or hidden surface without being a card. The registry still validates its existing live state and answer authority. Only a card surface carries an individually registered card type. An unknown card type is refused and logged; it cannot claim a status surface from an agent-supplied field.

An actual Room/client link loss reads Connection lost while the existing reconnect policy runs. Its existing give-up point offers Retry through the existing reconnect action. A late health probe is not a loss, and an already-resolved answer stays distinct from connection state. Neither condition creates a card.

Stopped notification subscriptions follow their canonical target: an Agent target shows an error in that agent's pane; a WorkflowEndpoint target shows the error in its owning Workflows/Sources view. No agent is inferred and no card is added (placement clarification248863).

A reached deadline shows `Time limit ended` as the agent tree warning and keeps the existing warning/status in the child pane, with no automatic card. The parent's normal-text time-limit notice is preserved (decisions248510/248939).
