# Linux native fixture contract

The test-only libc runtime emits exactly 30 distinct passing records, validated
by `linux-native-contract.mjs`. The two identity checks run inside the native
runtime after confinement. The external supervisor independently inspects all
seven namespaces, UID/GID, empty supplementary groups, zero capabilities,
seccomp, no-new-privileges, mounts, executable identity and the bounded service
cgroup before allowing the runtime constructor to execute.

Three controls are recorded separately from the 30 native checks:

- An executable data mount is rejected before readiness or runtime loading.
- Removing native policy while retaining bubblewrap exposes raw networking and
  fork permission; the probe must report both failures.
- Removing namespaces, read-only mounts and native policy exposes a readable
  owned host sentinel, a writable owned package and fork permission; the probe
  must report all three failures. This control directly launches the test-only
  weakened worker against fresh fixture directories. It does not read private
  host data or touch installed Apps.

The controls retain stdout so the failure receipts remain inspectable. The
weakened worker is never installed or accepted as a production runtime.
These checks qualify the production launcher with an unsigned libc fixture.
They do not qualify Node, signed installed artifacts, independent hostile App
packages or macOS. See `scripts/test-app-worker-linux-ci.sh` for the disposable
hosted-runner entry point. Builder execution must use a private VM rather than
modify shared host AppArmor or enrollment domains.
