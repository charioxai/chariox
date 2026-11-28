# Account machine limits (row 569)

Cloud owns account limits, currency and usage. The kernel owns authority for its agents and its own no-ask Settings. Account limits use the existing optional control-plane port; OSS has no supplier catalog, pricing or Cloud database logic.

## Surfaces and contract

`chariox.machines.limits` reads the account and no-ask limits. `chariox.machines.set_limits` takes `scope: account | no_ask`, optional `machines_at_once`, `spend_per_day_micros`, and `classes` for no-ask only. These tools are discovered on demand and listed only for a connected account. A service that offers no limit actions gives no account limit authority.

`chariox machine limits` reads. `--machines N`, `--spend AMOUNT` and `--scope account|no-ask` request changes; `--classes` is no-ask only. A refusal carries the owner's exact next step. Limits changes do not create or poll for a new approval card. Existing machine-acquisition decisions keep their own behavior.

`ControlPlaneRequest::Limits` reads values, currency, version and optional machine/spend usage. `SetLimits` carries the expected version and exact requested values. The kernel attaches an authorized approval; an agent cannot supply its own. Cloud authenticates the kernel and rechecks the expected version/from/to values before writing. Usage counters moving alone do not invalidate approval: equality covers limits, currency and version, not in-use/spend-today.

The client's Settings summary reads the same kernel port and labels the effective no-ask values when the account clamps them. A rejected read cannot authorize an acquire on a guess.

## Authority

A lower needs no extra authority. A raise by an agent needs a live window for the person's own `/sudo` prompt naming the scope and bounds:

- `/sudo raise account limits to 4 machines`
- `/sudo raise no-ask limits to 4 machines and 15 EUR a day`
- `/sudo raise no-ask limits to 4 machines and classes ["small","big"]`

The existing `raise limits ...` spelling names account scope. Numeric grants permit the named value and smaller raises, preserve currency checks and cover only named fields. Class authority names the exact case-sensitive target list; it grants no implicit or wildcard expansion. Account authority never grants no-ask changes, nor the reverse. One combined request must be covered by one grant before any write; alternating partial grants cannot accumulate authority.

The kernel parses the owner's prompt once at submission, stores structured `LimitGrant` on `KernelSudoTurn` in memory, and never rereads display text for authority. The field is skipped by serialization. An agent's request text creates no such grant. Current running-agent, prompt/window, owner, placement and expiry checks remain authoritative. A window that ended or belongs to another agent covers nothing.

Missing authority returns `owner_action_required` with the exact owner step. It creates no raised-limit card and cannot be approved through another agent or a generic Question. Sub-cent amounts that the existing two-decimal prompt grammar cannot name are never rounded upward: the refusal names the full value and the owner's direct setting path. The owner's own Settings changes stay available.

The source contract still represents approval kinds for request validation; externally supplied approvals, including a Card-shaped value, remain rejected for agent requests. Only a Sudo approval is produced by this limits path. Any paired Cloud contract cleanup is coordinated with its owner.

## No-ask bounds and writes

The effective no-ask machine/spend limits are the minimum of the owner's Settings and the account's bounds, in the account currency. Classes are checked against the connected service's offered classes. Unknown classes and classes in account scope are refused. A failed bound read never permits a no-ask acquire; the existing ordinary acquisition decision remains its fallback.

No-ask class and numeric changes are planned and authorized together. The write compares the current class/count/spend settings with the planned snapshot under the existing app mutex; a newer owner edit is never overwritten. Lowering one field cannot sneak through a rejected combined raise.

## Source and verification boundaries

Kernel code: `local/api/types/control_plane.rs`, `machine_acquire.rs`, `kernel_access.rs`, `runtime/control_plane.rs`, `runtime/state/machine_acquisition/limits.rs`, `without_asking.rs`, and the existing sudo entry/structured-grant modules. CLI and kernel-client request/Settings helpers use that same port. Serialized contract/version/snapshot work remains joint protocol-owner work; no historical hash or version is authoritative for a newly composed head.

The source tests cover reads, direct lowering, exact scoped numeric/class grants, insufficient/ended/wrong-agent windows, forged approval rejection, no new card/no write on refusal, combined atomicity, stale versions/settings, moving usage counters and the no-ask clamp. The stale-write fixture polls the real unspawned transaction while the app mutex is held, applies the owner's normal app setting, then requires refusal; it uses no timer race, raw store write or production hook.

Acceptance must use an identified owned staging kernel and the paired Cloud source through the public owner/agent paths. Read/Settings/authority cells do not require a new machine. Physical acquisition/lifecycle cells need their own assigned scope; no such action follows from this document. Source fixtures and syntax checks are not live or current-head acceptance.
