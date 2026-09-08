The Linux root installer accepts a separately trusted Ed25519 public key and
the expected inventory SHA-256. Neither value is read from the artifact as
authority. It does not execute the runtime or attest its sandbox.

```sh
chariox-app-runtime-install install --source /absolute/signed-bundle \
  --trusted-public-key-hex <64-lowercase-hex> --inventory-sha256 <64-lowercase-hex>
chariox-app-runtime-install cleanup --inventory-sha256 <inactive-inventory-sha256>
```

The executable requires real and effective UID 0 and has no output-path or
environment override. Its only production destinations are
`/usr/lib/chariox/app-runtimes/<inventory-sha256>` and
`/etc/chariox/apps/runtime-enrollment.json`. macOS installation is unsupported;
the private filesystem tests do not provide a production bypass.

One persistent installer lock serializes publication and recovery. All traversal
and mutation uses anchored no-follow directory descriptors. The input owner's
UID constrains traversal; the exact external key and digest establish trust.
The same verifier checks the signed fixed graph before copying and checks the
copied graph again before publication. Copies stop at each signed size; the
whole graph is at most 512 MiB. Publication also requires 128 MiB free reserve.
At most eight generations are retained, including any inactive generations.

Payload files are 0444, native executables 0555, and published directories 0555.
A staging root remains writable until exclusive rename, then it is sealed and
the parent directory is synced. Enrollment is published only afterward by
synced-file/atomic-rename/directory-sync. A new key or inventory increments the
enrollment revision; an exact retry re-verifies the installed graph and keeps
its revision. A key rotation requires a new inventory digest: the signature of
an existing immutable generation is never replaced in place. Revisions never wrap.
Recovery syncs both authorities before
reading or acknowledging a visible prior rename. A failed publication does not
implicitly roll back an enrollment rename that may already have become durable.

Old generations remain available to readers holding their shared runtime lease.
Cleanup refuses the enrolled generation and requires an exclusive lease. It
renames an inactive generation into a durable retirement location, deletes only
fixed graph entries, and removes the lease last. The held exclusive lease is
released after directory removal and parent sync. Interrupted cleanup resumes
under the same installer lock and lease; an active reader produces Busy rather
than an acknowledgment of completed cleanup. Staging and retirement recovery
never deletes unknown entries or follows links.

The root installer/OS package must deliver this binary and supply the trusted
key and expected digest from its release authority. Automatic OS packaging and
signed macOS installation remain separate integration work. Unit fixtures use
tiny signed files in a private external test directory, inject publication and
cleanup interruptions, and run no privileged tools or native App code.
