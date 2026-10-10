# amaro 1.1.5 (vendored)

`index.js` is `dist/index.js` from the npm package `amaro@1.1.5`
(`sha512-oo72OEYOSfSPe+96V+jh41gaFWfl9HddXFAHMFM+emjdZkzbtcg0bFWoYaWf8IMlqmZ0VM8F13frI8Ktt3+ADA==`),
unchanged. It is byte-identical to the TypeScript stripper that official Node.js
22.22.1 embeds as `internal/deps/amaro/dist/index`. Licenses: `LICENSE.md`
(amaro, MIT) and `LICENSE-swc` (SWC, Apache-2.0).

MP-08 / MP-11: the kernel sends this file to the isolated workflow compiler for
TypeScript sources, so every Node build (including distribution builds without
native type stripping) strips types inside the isolation boundary exactly like
official Node. To update, replace the files from the matching npm tarball and the
pinned SHA-256 in `workflow_code/tests/compiler.rs`.
