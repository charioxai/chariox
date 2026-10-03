# Native release build resources

The production `scripts/build-managed-kernel-release.mjs` defaults to one Cargo
job. `--cargo-jobs 2` is opt-in and requires every verified BuildKit node to have
at least four bounded CPUs and 12 GiB bounded RAM. Existing maximum resource and
finite swap/PID limits still apply. Other job counts are rejected. The release
Dockerfile also rejects values other than one or two; development builds retain
their existing one-job setting.

Only resize an existing builder after all prior builds have terminal settlement.
Changing limits during a build invalidates its retained builder fingerprint.
Resource updates need not restart the builder or remove its cache. The operator
must check host memory, CPU and disk headroom first. This option does not replace
the source-bound attestation, exclusive builder ownership, build deadline or
remote cancellation settlement. Build the new reviewed source revision after
changing this Dockerfile; never attach old artifacts to a new attestation.
