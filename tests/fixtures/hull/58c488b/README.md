# hull's shared CircleCI files, vendored

Copies of hull's files at commit `58c488b` (keelstackhq/hull `origin/main`: HU-3 #46, #45, #48,
#49). No hull tag has them yet; v0.3.1 predates HU-3. `MANIFEST` lists each file at its hull path.
`tests/channel_render/circleci.rs` checks that the channel renders them byte-identically when
`own_ci` is empty.

TODO(hull-release): re-vendor at the next hull tag. Copy each `MANIFEST` path from the tag into
`tests/fixtures/hull/<tag>/`, move `MANIFEST` (with the new commit) and this file there, delete
this directory and set `HULL_REF` in the test to the tag.
