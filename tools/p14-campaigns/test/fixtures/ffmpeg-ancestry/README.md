Synthetic fixtures for `ffmpeg-ancestry.test.cjs`: invented identifiers, hashes and patches in the shape of
the National Vulnerability Database's and GitHub's responses. `records.json` holds six records (a cherry-pick with
an identical patch, an ancestor named by a gitweb URL, a cherry-pick named by an abbreviated hash whose patch differs
and which a later commit reverts in the candidate, a record with no commit, a commit the mirror does not hold, and
an absent fix);
`github.json` maps each request path to its response. No test touches the network.
