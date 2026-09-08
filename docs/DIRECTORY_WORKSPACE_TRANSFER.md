# Directory workspace transfer

Managed development-context transfer supports a selected directory without Git.
Export and import must not initialize a repository or synthesize a commit. The
same context plan, encrypted transfer, publication, resource budgets and receipt
recovery paths apply to Git and directory workspaces.

## Representation

- Git-only archives and receipts retain schema2 and omit workspace-kind fields.
- A directory workspace uses kind `directory`, schema3, empty Git bundle and HEAD
  fields, checked file objects, and an explicit directory list for empty folders.
- Imported workspace and launch records retain the kind, including across kernel
  restart. A directory HEAD must be empty; a Git HEAD must remain a valid40- or
  64-digit hexadecimal object ID.
- Local messages encode `workspaceKind`; relay records encode `workspace_kind`.
  An omitted kind means Git for existing records. Shared daemon protocol318
  versions the new shape. Older importers must reject unsupported schema3 rather
  than silently treating directories as repositories.

## Filesystem policy

Directory export reuses the forced exclusions, `.charioxignore` matching, file
hashing, permissions and materialization limits used by Git overlays. It rejects
symlinks and special files, uses a retained root descriptor with no-follow
component opens, and compares two bounded snapshots before publication. Import
validates the manifest and file objects before publishing into an owned private
destination. Empty directories and executable file bits are retained.

The secure descriptor-based implementation currently targets Unix hosts. Other
hosts fail explicitly; they do not fall back to following paths unsafely.

## Acceptance

Focused tests cover plain export/import without `.git`, pruned receipt recovery,
exclusions, symlink/special-file rejection, malformed metadata, mixed launch
records across restart, and local/relay serialization. Existing Git development,
transfer-state and protocol regressions must stay green.

These local tests are not evidence of managed-machine acceptance. A Web-terminal
launch on the managed machine, mixed-project transfer and interrupted/retried
publication drills remain necessary before claiming the complete product flow.
