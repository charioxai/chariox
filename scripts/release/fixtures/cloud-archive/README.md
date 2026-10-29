# MP-08 / MP-09 / MP-10 / MP-11 archive trust regression

These are verbatim Cloud files, retained so the OSS regression runs without a
private Cloud checkout. No fixture code is substituted for the archive chain.

| Fixture | Cloud commit | Original path |
| --- | --- | --- |
| `main-create.sh` | `8bbdf60b3c` | `deploy/openship/create-cloud-staging-release.sh` |
| `promotion-create.sh` | `369e65eee7` | `deploy/openship/create-cloud-staging-release.sh` |
| `extract-cloud-staging-release.py` | `8bbdf60b3c` | `deploy/openship/extract-cloud-staging-release.py` |
| `release-source-digest.mjs` | `8bbdf60b3c` | `deploy/openship/release-source-digest.mjs` |

The extractor and digest are unchanged between those commits. The main helper
executes the selected source's digest script on the host; the promotion helper
uses its own trusted directory. Keep the rejected main helper unchanged.
