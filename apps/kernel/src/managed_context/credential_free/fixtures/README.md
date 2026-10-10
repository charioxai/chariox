MP-08 / MP-11 public scanner regressions

slugify-index.d.ts.txt is the unchanged index.d.ts blob 7693318b9206d0f2d925b52fcc0c1564e9b88824 from sindresorhus/slugify, selected at 3b17b2e84b97624a683aafaa38184bf2746fab22 (MIT). requests-commit-message.txt is the message of public psf/requests commit c0813a2d910ea6b4f8438b91d315b8d181302356; it contains a documented timeout assignment in backticks (Requests source is Apache-2.0). These are public prose/source fixtures, with no source-private credentials.

Both fail the previous detector. Tests require ordinary source to pass and credential assignments, dynamic sensitive names, and private-key markers appended to the same source to fail. No public-origin, extension, fixture-directory or content-digest credential exemption is implemented. Requests' complete history/private-key fixtures and unsupported binary files remain subject to the existing guards.
