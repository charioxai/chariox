# Check kernel state before a binary upgrade

1. Keep the installed kernel binary in place. Stage the candidate and verify
   its release signature and checksums using the usual deploy procedure.
2. Prepare a consistent, private copy of the kernel home. Use a storage
   backup of the stopped kernel or a point-in-time filesystem backup. Include
   the state database and any SQLite write-ahead log or rollback journal from
   the same point in time. Keep the kernel configuration and registry with
   the copy. If `state.path` or `CHARIOX_PUBLICATION_CONTROL_STATE_DIR`
   points at state outside the copied home, point it at the copied database.
3. Immediately before switching the binary, run the **candidate** against
   the copied home:

   ```sh
   /path/to/candidate/chariox-kernel --check-state --home /path/to/copied-kernel-home
   ```

   Use the installed kernel's `CHARIOX_KERNEL_HOST` and `CHARIOX_KERNEL_PORT`
   if it uses a custom endpoint. If its service sets `CHARIOX_DAEMON_ID`,
   use that same value. With no `--home`, the check uses the usual
   `CHARIOX_HOME`, XDG, or home directory configuration.

   | Exit | Result | Next step |
   | --- | --- | --- |
   | 0 | State replay passed. | Switch to the staged binary and verify startup through the usual deploy procedure. |
   | 1 | State replay failed. | Keep the installed binary. Share the reason and the record kind, ID, and sequence, when reported, with the release maintainer. |
   | 2 | The check could not run. | Correct the reported path, permissions, or temporary storage problem and retry. |

The check runs the candidate's durable checkpoint and event replay. It uses
private temporary storage for database recovery and migration. The input
remains read-only, and the check does not start the kernel, listen on sockets,
connect to a relay, or open the Vault. Allow enough space in `TMPDIR` for the
database and SQLite side files. Complete the usual startup and connection
checks after the switch.
