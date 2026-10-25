## Running kernel tests from a lanes-kernel worker (loop rule)

You run under Chariox, so your shell has a Claude provider process among its ancestors. Kernel tests that check the
process ancestry (for example `runtime::kernel_access::requester::launcher_tests::*`, among them
`unknown_requester_does_not_select_a_provider_sibling`) then fail for a reason that has nothing to do with your change.

For any cargo run that includes those tests, start the command in its own session and read its result from files:

```sh
out=<your lane directory>/tmp/kernel-tests.log
setsid -f bash -c 'cargo-slot.sh cargo test -p chariox-kernel --lib <filter> >"$0" 2>&1; echo $? >"$0.rc"' "$out"
until [ -e "$out.rc" ]; do sleep 5; done; tail -n 40 "$out"; cat "$out.rc"
```

Run only the commands that need it this way; everything else keeps its normal foreground form. The detached command
ends by itself; do not leave it running when you finish, and signal it only by an exact PID you recorded.
