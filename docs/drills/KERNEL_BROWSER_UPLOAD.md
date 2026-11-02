# Kernel browser file upload (MP-08 / MP-09 / MP-10 / MP-11)

The agent's `chariox.kernel_browser` tool accepts uploads on its existing
document-bound input lane:

```json
{
  "document_id": "<observed document_id>",
  "command": {
    "op": "input",
    "tab_id": "<observed tab_id>",
    "generation": 1,
    "input": {
      "kind": "upload",
      "target": "backend:7",
      "paths": ["report.txt", "images/chart.png"]
    }
  }
}
```

Use the actual tab, generation, document and file input's `node_ref` from a
fresh snapshot. For one file, `path` replaces `paths`. Relative paths resolve
against the Room worktree; absolute paths must remain inside it. The kernel
supplies that root after the ordinary user-domain grant check. A caller cannot
supply roots. Existing grants have no directory resource, so this operation
does not add a grant for arbitrary host directories.

The operation allows 1–20 regular files, at most 512 MiB total. It refuses `..`,
paths outside the worktree and symlinks escaping it with the existing
`user_domain_not_granted` reason. Missing files, directories, special files,
oversized requests and invalid targets produce bounded, plain reasons. Only
an actual `<input type="file">` is supported. Chooser buttons and drop zones
are outside this operation's scope. Selecting several files requires `multiple`.

The existing stager copies approved file descriptors before CDP sees them;
changing a source path cannot redirect Chromium's later reads. Host CDP pipes
use their own browser identity. Their staged files survive connection loss and
remain bounded by the existing quota; the owned browser shutdown path reclaims
them after retirement. An unproven retirement retains files and reports the
storage problem. The stager currently requires Linux descriptor resolution.

Uploads retain the usual document, grant, cancellation, App-view and human
takeover checks. Success adds `upload: {file_count, total_bytes}` to the JSON
state result; paths and bytes do not enter the action ledger. This is a tool
JSON extension, without a change to typed local or relay requests.

Run the focused regressions:

```sh
node --test --test-concurrency=2 \
  apps/kernel/slice-linux-docker/docker/kernel-browser-upload.test.mjs \
  apps/kernel/slice-linux-docker/docker/browser-controller-files.test.mjs \
  apps/kernel/slice-linux-docker/docker/browser-controller-upload-binding.test.mjs
```

For the supplementary native-browser regression, run as a normal Unix user
with private, writable HOME/XDG/TMPDIR paths and an installed Chromium:

```sh
CHARIOX_KERNEL_BROWSER_EXECUTABLE=/absolute/path/to/chrome \
CHARIOX_KERNEL_BROWSER_HEADLESS=1 \
CHARIOX_KERNEL_BROWSER_MIRROR=0 CHARIOX_KERNEL_BROWSER_DISPLAY=0 \
node --test apps/kernel/slice-linux-docker/docker/kernel-browser-upload.browser-test.mjs
```

The native test snapshots the input, selects workspace files, submits a real
multipart form and compares the server's file names, sizes and SHA-256 values.
It also checks outside-path refusal, non-file controls and staging cleanup.
`CHARIOX_UPLOAD_SCREENSHOT` optionally saves the resulting page. On a host whose
system temporary directory is not writable, set
`CHARIOX_BROWSER_LIFECYCLE_ROOT` to another private writable directory as well.

This regression supplements the real kernel/provider drill: start a disposable
kernel and real TUI, select an existing product-linked Codex account with
`gpt-6-luna` / `low`, and ask the agent to perform the same snapshot → upload →
submit flow using only the browser tool. Record the server echo, outside-path
refusal, provider transcript, screenshots, exact source/binary identities and
cleanup outside the repository. A local result does not close the hosted or
managed-machine parity matrix.
