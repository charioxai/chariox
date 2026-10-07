# Sandboxed host Chromium on Ubuntu

On Ubuntu 24.04 and later, a portable Chromium can fail with “No usable
sandbox” when `kernel.apparmor_restrict_unprivileged_userns=1`. The kernel
reports this restriction and the supported fixes when Chromium fails to
start. The flag alone does not imply failure: an installed browser with an
appropriate profile or a working SUID sandbox may start normally.

Use the system browser installed with its sandbox support. For Google Chrome
stable in its normal package location, launch the kernel as your ordinary user:

```bash
cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns
test -x /opt/google/chrome/chrome
export CHARIOX_KERNEL_BROWSER_EXECUTABLE=/opt/google/chrome/chrome
export CHARIOX_KERNEL_BROWSER_HEADLESS=1
# Start your normal kernel using its existing CHARIOX_HOME and relay profile.
chariox-kernel
```

Ubuntu's packaged Chromium is another option when installed with its sandbox
and AppArmor profile. Set `CHARIOX_KERNEL_BROWSER_EXECUTABLE` to its absolute
launcher path. Keep Chromium in its installed location: copying the browser
into a temporary directory can remove the path-based AppArmor admission.

For a portable build, an administrator/installer must provision either an
AppArmor profile permitting `userns` for that specific executable or a
compatible, correctly installed SUID sandbox helper. The kernel preserves
`CHROME_DEVEL_SANDBOX` when explicitly configured by the operator; it does
not install privileged helpers or change host policies. Prefer root-owned,
immutable executable paths over broad writable-path profiles. See Chromium's
[AppArmor/userns guidance](https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md)
and [SUID sandbox installation](https://chromium.googlesource.com/chromium/src/+/main/docs/linux/suid_sandbox_development.md).

Keep the host restriction and browser sandbox enabled. Chariox never retries
with `--no-sandbox` or `--disable-setuid-sandbox`, and does not silently switch
executables after a launch failure. Browser stderr stays private; only fixed
actionable diagnostics cross the kernel browser response boundary.

Regression: `scripts/e2e-stack/check-host-sandbox.mjs` in Cloud runs the actual
controller on an Ubuntu host as a normal user, without container overrides.
Run `restricted` with the portable browser against both base and fixed assets;
run `ready` with the installed system browser. Follow with the real built web
entry, device pairing and hosted relay drill; the controller regression alone
is not live product acceptance.
