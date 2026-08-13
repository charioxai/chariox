# M18 Slice Saved State Plan

## Goal

Allow users and authorized agents to save the current state of a slice and later launch a slice from that saved state, without exposing Docker image, container, or volume lifecycle details to the user.

The default product concept is **saved slice state**, not a visible list of checkpoint versions. A user should be able to click `Save state`, and Chariox should make the slice reusable with the same browser logins, local configuration, installed tools, and runtime environment.

Agents should be able to request the same operation through the runtime MCP. The home kernel remains the authority that performs the host-side Docker operations.

## Product Model

Default operation:

```text
Save state
```

This overwrites the active saved state for that slice/profile.

Backup operation:

```text
Create backup
```

This writes a separate backup copy, but does not make Chariox manage multiple visible versions in normal UX. After backup creation, Chariox shows where it lives and explains how to manually swap it in if needed.

User-facing concepts:

```text
Save state         -> overwrite current reusable slice state
Create backup     -> copy current saved state somewhere safe
Reset state        -> go back to the base Chariox slice image/profile
Launch from state  -> create/start a slice using saved state
```

Internally, saved state is composite:

```text
saved-state/
  manifest.json
  image_ref
  home.tar.zst
```

Both pieces are required because Docker image state and `/home/slice` volume state are separate. `docker commit` captures container filesystem changes outside mounted volumes, while browser profiles, Gmail cookies, provider auth files, user config, and many app caches live in the `/home/slice` Docker volume.

## Storage Layout

Use a Chariox-managed state root under the existing slice root.

```text
~/.chariox/slices/states/<state_id>/
  manifest.json
  home.tar.zst

~/.chariox/slices/backups/<backup_id>/
  manifest.json
  home.tar.zst
```

Docker images:

```text
chariox-slice-state:<state_id>
chariox-slice-backup:<backup_id>
```

For a slice named `gmail-work`, the active state can be deterministic:

```text
state_id = gmail-work
image_ref = chariox-slice-state:gmail-work
```

Backup names can be timestamped:

```text
gmail-work-20260609-181500
```

## Protocol And API

Add local daemon requests:

```text
slice.state.save { slice_ref }
slice.state.status { slice_ref }
slice.state.reset { slice_ref }
slice.state.backup.create { slice_ref, name? }
```

Extend slice creation:

```text
slice.create {
  name,
  ...
  from_saved_state?: string
}
```

Protocol changes must follow the protocol change rule:

- bump the local daemon protocol version;
- update protocol shape/hash tests;
- update protocol docs for slice lifecycle and saved state behavior;
- add or update a focused drill.

## Kernel Model

Add fields to `SliceRecord` or a sibling saved-state store:

```text
saved_state_ref?: String
saved_state_status?: none | saved | missing | failed
saved_state_updated_at_ms?: u64
```

Add `SliceSavedStateRecord`:

```text
id
slice_name
source_slice_id
backend
os
image_ref
home_archive_path
created_at_ms
updated_at_ms
last_operation
last_operation_status
last_error
```

Add `SliceBackupRecord`:

```text
id
name
source_slice_id
source_state_id
image_ref
home_archive_path
created_at_ms
size_bytes?
```

Persist records through durable events and include slice audit events. Saved state and backup artifacts are secret-bearing because they may contain browser sessions, provider auth material, and account cookies. Do not expose archive contents to provider transcripts.

## Docker Backend

Add local Docker actions:

```text
save-state
restore-state
reset-state
create-backup
delete-state
```

Save state:

1. Resolve the slice container and `/home/slice` volume.
2. Require no active agents.
3. Stop/quiesce the container.
4. Run `docker commit <container> chariox-slice-state:<state_id>`.
5. Archive the home volume to `states/<state_id>/home.tar.zst`.
6. Write `manifest.json`.
7. Mark state saved and audit success/failure.

Restore state during slice create/start:

1. Use saved `image_ref` instead of the configured base image.
2. Create a fresh home volume.
3. Extract `home.tar.zst` into that volume before first start.
4. Continue the normal provision/start path with new ports, relay identity, and worker kernel identity.

Reset state:

1. Remove active state image.
2. Remove active home archive.
3. Clear saved state metadata.
4. Future slice launches use the base image/profile.

Create backup:

1. Copy active state image to a backup image tag, or commit the current container if requested from a live slice.
2. Copy/export the home archive into `backups/<backup_id>/home.tar.zst`.
3. Write backup manifest.
4. Show a message with the backup path and manual swap instructions.

## UI, CLI, And Slash Commands

Web terminal buttons:

```text
Save state
Create backup
Reset state
```

TUI slash commands:

```text
/slice save-state
/slice backup
/slice reset-state
/slice state
```

CLI:

```text
chariox slice state save <slice>
chariox slice state status <slice>
chariox slice state reset <slice>
chariox slice backup create <slice> --name gmail-ready
```

After backup creation, show:

```text
Backup saved:
  ~/.chariox/slices/backups/gmail-work-20260609-181500

To use it manually, stop Chariox slice operations, then swap this backup directory
with the active state directory:
  ~/.chariox/slices/states/gmail-work

The Docker image tag for this backup is:
  chariox-slice-backup:gmail-work-20260609-181500
```

First-class restore-from-backup can be added later, but is not required for the initial feature.

## Runtime MCP

Expose:

```text
chariox.save_slice_state({ slice_ref? })
chariox.create_slice_backup({ slice_ref?, name? })
```

If `slice_ref` is omitted, infer the current agent's slice.

Authorization follows the existing agent rights taxonomy:

- user: always allowed;
- full-rights agent: allowed directly;
- permission-required agent: route through the existing approval flow.

The runtime MCP returns only status, state id, backup id, image refs, and paths. It never exposes checkpoint archive contents or secret-bearing files.

The in-slice agent must not receive the host Docker socket or direct host Docker authority. It requests the save through the runtime MCP; the home kernel performs the operation.

## Lifecycle Rules

For the first implementation, checkpointing requires no active agents on the slice. If the slice is running but has no active agents, the kernel stops it cleanly, saves state, and leaves it stopped.

This is simpler and more deterministic than live checkpointing. A later implementation can add `restart_after: true` if needed.

## Tests And Drills

Unit tests:

- saved state record creation/update;
- backup record creation;
- reset clears saved state;
- protocol version and shape tests;
- authorization tests for user, full-rights agent, and permission-required agent;
- `slice.create` selects saved-state image/home restore metadata.

Local Docker drill:

1. Launch a headed slice.
2. Create a marker in `/home/slice`.
3. Create a marker outside `/home/slice`.
4. Save state.
5. Delete original slice/container.
6. Launch a new slice from saved state.
7. Verify both markers exist.

Browser state drill:

1. Log into Gmail or a lower-friction authenticated test service.
2. Save state.
3. Launch a new slice from saved state.
4. Verify the browser remains authenticated.
5. Save screenshot proof in `.artifacts`.

Backup drill:

1. Create backup.
2. Verify backup directory and image exist.
3. Verify surfaced instructions are correct.
4. Optionally manually swap backup into active state and relaunch.
