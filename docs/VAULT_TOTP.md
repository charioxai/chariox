# TOTP credentials (MP-08 / MP-09 / MP-10 / MP-11)

The kernel stores an authenticator seed in the encrypted Vault and computes
RFC 6238 codes when it injects a credential. The agent receives the credential
handle and the result of the input operation. It never needs to read or copy a
code. Local/peer protocol versions are 945/394.

Use the existing protected `chariox.request_credential_secret` interaction with
this credential configuration (the seed is entered by the owner, outside the
agent's tool arguments):

```json
{
  "id": "example-authenticator",
  "allowed_hosts": ["example.com"],
  "allowed_uses": ["browser", "pty", "env"],
  "injection": {
    "kind": "totp",
    "algorithm": "sha1",
    "digits": 6,
    "period_s": 30
  }
}
```

The protected value may be a base32 manual key or an `otpauth://totp` URI.
URI provisioning parameters replace the corresponding configuration fields;
the Vault stores a canonical base32 seed and the registry stores only the
algorithm, digits and period. SHA-1, SHA-256 and SHA-512 and 6/8 digit codes are
supported. Input validation never includes the supplied key in an error.

Fill the observed code field with `chariox.kernel_browser_paste_secret` and
`submit=false`, then click the observed form button. Browser grants, document
bindings, host restrictions, Vault unlocking and target masking are the same
as for password input. The current code is registered for protection before
insertion. Terminal stdin uses `chariox.send_secret_to_terminal`.
TOTP and recovery-code credentials accept editable text, telephone and numeric
inputs as well as password inputs. The kernel marks this internal fill path
only after validating the factor handle and registers the target before input.
The owner's field keeps its original type; model snapshots and screenshots
mask the Vault-filled target. Other credential kinds keep their existing
input rules.
`RuntimeSecretService::environment_secret_input` is the resolver for S2's
env executor; the env execution tool is supplied by S2, not this slice.

Factor capture and input require a live sudo window for the exact current
agent task. `vault_elevation_allows` is the single policy seam for replacing
this check with project-mandate-or-sudo authorization when S1 lands. The
kernel checks again after waits and at the final browser/terminal input and
storage boundaries. Owner credential storage does not need agent elevation.

Store recovery codes under a separate handle with `injection.kind` set to
`backup_codes`. Enter one code per line or a JSON array of strings through the
protected interaction. The encrypted value includes consumption state; no code
is kept in public metadata. Every input takes one unused code and durably marks
it used before dispatch, under a kernel lock shared by service instances.
An ambiguous or failed delivery can therefore cost one code. Once exhausted,
the handle fails with a request to enroll a new set. Policy checks do not
consume codes. Explicit overwrite replaces the set.

QR decoding and atomic browser capture of service-created keys belong to S3.
The new-UI Vault window is developed separately; this change adds no client
controls. `scripts/e2e-stack/totp` supplies the supplementary local enrollment
site and real-provider drill. A local fixture result alone does not close the
real-service `apps-autonomy/AU-10` acceptance row.
