# ADR 0012: P04 private snapshot and restricted media profile

- Status: Accepted
- Date: 2026-09-12
- Scope: P04 / issue #7; does not enable a public session command or strict worker

## Context

FFmpeg cannot consume the held `cap-std` source file on every target through the
existing null-stdin process supervisor. Handing it a mutable original path would
weaken source identity and allow path substitution between VSift's read and decode.
FFmpeg protocol flags alone do not provide filesystem or memory isolation on a
desktop host. P04 also needs a usable, measurable media subset before P06 pins and
provisions specific provider builds.

## Decision

P04 always stages a bounded local regular file into an initialized, private P03
session artifact directory. The original is opened no-follow relative to a held
parent; the copy is SHA-256 identified, rehashed before each provider call, and held
under the session lifetime and weighted admission controls. An explicit source-change
query rehashes the original. This is the private-snapshot source policy from the
architecture contract; P05 will own lifecycle and publication.

The initial adapter admits ISO base media and Matroska/WebM magic only, with forced
`mov` or `matroska` demuxers, the `file` input protocol, null stdin, empty child
environment, fixed arguments and stdout-only derived output. MOV external data
references and absolute aliases are explicitly disabled. HLS, concat, playlists,
arbitrary URLs and provider-supplied filter/option text are rejected. P02 supervises
the process tree, operation deadline, cancellation and separately capped output and
diagnostic streams. The profile limits source bytes/time, probe bytes, stream count,
duration, displayed pixels, PNG bytes and PCM range/bytes.

Stream indexes remain explicit. Normalized source time subtracts the earliest stream
start. MP4 reports duration as a span; Matroska with a timestamp offset reports a
container end PTS, so its origin is subtracted. Frame selection returns the first
displayed frame at or after a request within tolerance, using the selected frame's
observed PTS. Rotation is reported in displayed coordinates; FFprobe's
counter-clockwise display-matrix angle is converted into the domain's clockwise
orientation. Audio reports the first observed decoded sample PTS, including codec
priming or source offsets.

`MediaProviderConformance` records the fixed P04 policy and trusted executable
provenance. It does not declare every FFmpeg build conformant. The opt-in mechanical
checkpoint records actual FFmpeg/FFprobe versions and runs the selected builds
against project-owned media; P06 will own immutable managed runtime identity.

## Limits and consequences

Desktop P02 process containment is not a network/filesystem/memory sandbox. A
same-user actor can modify a private snapshot after verification; no metadata check
or read-only bit can fully defeat that actor. A malicious decoder may allocate
multiple buffers under the per-allocation limit before the watchdog stops it.
Strict worker filesystem/network/resource isolation remains P11/P14 work and must
fail closed until inherited host controls are qualified. P04 therefore makes no
whole-machine sandbox or durable-storage claim.

The corpus generator emits visual evidence and tone sentinel audio. Spoken scripts,
local ASR and source-inclusive session publication remain P07/P05 work. F11's safe
malformed/parser variants exercise P04 rejection; dangerous decompression-bomb
variants require an owned disposable limited environment in P14.

## Alternatives

- Bind the original path directly: rejected because the provider cannot consume the
  held source handle and a mutable path loses stable source identity.
- Accept arbitrary FFmpeg demuxers or URLs: rejected because external references
  and provider-specific I/O would cross the reviewed input boundary.
- Require strict worker isolation for desktop P04: rejected because ADR 0005 keeps
  desktop and worker guarantees distinct; P11 has not qualified that host boundary.

## 2026-09-26 implementation note: bracketed binding for multi-call operations

Issue #148, P08. The per-call rehash above is linear in source size for every
provider call, and local speech recognition (ADR 0017) makes one `FFmpeg` call per
30 s chunk: a one-hour source was hashed about 150 times. Operations that call a
provider more than once now bind the private copy for the whole operation instead:

- **Open:** `BoundSource` reopens the committed copy no-follow and verifies its
  SHA-256 once, recording the on-disk identity of the file it hashed (read from the
  same handle before and after the bytes, so a write during the hash fails).
- **Before each provider call:** the copy is reopened by name and only its identity
  is compared: size, modification time, device and file index on every platform,
  plus the change fields the media-tool fingerprint already uses (Unix permission
  bits, owner and status-change time; Windows creation time and attributes).
- **Before commit:** one last identity comparison and a second full SHA-256
  verification. Only then is the snapshot handed back to the caller for commit.

The media adapter takes a sealed `SourceBinding`. A plain `SourceSnapshot` keeps
this ADR's per-call rehash for single-call operations (`ingest`'s probe, the
media-tool fixture check); a `BoundSource` compares identity. The speech-audio
adapter accepts only a `BoundSource`, and `transcript retranscribe` and the local-ASR
fixture check use one, so a run hashes the copy exactly twice however many chunks it
decodes. A mismatch uses the existing typed mappings and commits nothing: at the
probe `INVALID_SOURCE`, at a chunk's decode `STORAGE_IO` (the source copy could not
be read, ADR 0017 section 8), and at the closing verification `INTEGRITY_FAILURE`.
No failure code was added. The closing verification is new: before, a run's last
check was the rehash before its last chunk, so a change after it went unseen.

**The residual in "Limits and consequences" is unchanged.** Per-call hashing proves
the bytes only at the instant they are hashed; a same-user actor could already change
the copy between that hash and the provider's read and restore it afterwards. The
bracketed binding has exactly that blind spot and no wider one: any change still
present when the operation ends fails the closing verification; a replaced file, a
changed size or modification time, and on Unix any write or metadata change (the
kernel sets the status-change time, which an unprivileged process cannot set back)
fail the next identity check before the provider reads the file. What remains is a
transient in-place change undone before the closing verification, and on Windows,
which has no user-immutable change time, a rewrite that also restores the
modification time, which only the closing hash sees. File times have the
filesystem clock's granularity, so a write within the same tick as the binding is
likewise left to the closing hash. The copy stays in a private session directory
under the session's lifetime hold, so only a same-user actor can write it at all.

Measured on Windows 11 (Xeon E5-2698 v4, release build) over an 869 MB, 10-minute
source of 24 chunks, probing and decoding every chunk took 25.5 s bound (2 full
hashes) against 173.4 s with the per-call rehash (26 full hashes). Holding a
deny-write handle for the run, or giving `FFmpeg` an open handle instead of a path,
were the other options in #148; neither is needed for this guarantee, and the
second is not portable through the process supervisor. Regression tests:
`crates/vsift-infrastructure/tests/p08_source_binding.rs`, the hash-count tests in
`crates/vsift-infrastructure/src/source_binding/tests.rs` and the opt-in engine
tests in `crates/vsift/tests/engine_retranscribe.rs`. Closes #148.

## 2026-09-26 implementation note: provider diagnostics parsing hardened (SEC-17)

P09 PR 1. "Frame selection ... using the selected frame's observed PTS" and "Audio
reports the first observed decoded sample PTS" above rely on reading `FFmpeg`'s
`showinfo` and `ashowinfo` diagnostics. The frame and audio readers accepted any line
that *contained* the filter's marker. At `-loglevel info` `FFmpeg` also echoes the
input's and the output's metadata, which the source author controls, so a crafted
`title` could supply the frame timestamp and time base, or the first audio sample time,
that the adapter then reported as observed. On the pre-fix code the regression tests
read pts 99999 in a 1/1 time base instead of 10752 in 1/10240, and 9.5 s instead of
64 ms. The P08 visual-sample reader already read only lines that begin with the marker
and was not affected. On user media the audio reader was reachable through `transcript
retranscribe`, where each chunk's first decoded sample time places its segments on the
timeline, so a crafted file could shift its own transcript's times; the frame reader
and the short-clip reader ran only on the embedded F01 fixture of the media-tool
preflight.

- **Line origin:** every reader now reads only lines that *begin* with the filter's own
  `[Parsed_showinfo_` or `[Parsed_ashowinfo_` prefix. `FFmpeg` indents every echoed
  metadata value and turns a line break inside a value into another indented line.
- **Consistency:** frame lines must be numbered `0, 1, 2, ...` without a gap or repeat
  and their timestamps must strictly increase, so a line that does begin with the prefix
  but is not the filter's own (for example one forged through a log message that embeds
  an untrusted string with a line break) adds a frame the real numbering does not have,
  and the output is rejected. A single-frame extraction now passes only the first
  matching frame through the filter (`isnan(prev_selected_t)`) and requires exactly one
  frame line.
- **Time base:** the single-frame call and the new evidence calls select by an integer
  stream-timestamp bound and fail closed unless the filter's `config in time_base`
  equals the probed stream's (`MediaError::TimeBaseMismatch`).

The parsers are published (`parse_frame_showinfo`, `parse_ashowinfo_start`,
`parse_frame_listing`) and fuzzed (`frame_showinfo`, `frame_listing`), including an
invariant that indented copies of every line never change a result. Regression tests:
`echoed_metadata_cannot_forge_a_frame_time` and
`echoed_metadata_cannot_forge_an_audio_start` (unit), `p09_recorded_diagnostics`
(real `FFmpeg` 9.0 output of a clip with a forged title), and the opt-in
`forged_metadata_cannot_move_a_reported_time`. No failure code changed. See
[ADR 0019](0019-evidence-navigation.md).

## 2026-09-26 implementation note: evidence calls compare the copy's identity across calls (ADR 0019 D1)

Read-only evidence calls (P09) bind the copy with `BoundSource::open_for_evidence`.
When the session's committed manifest records a verified identity of the copy (a
digest of its size, modification time, device, file index and platform fields: Unix
mode, owner and status-change time; Windows creation time and attributes) and the copy
still has exactly that identity, its bytes are not hashed. Otherwise it is hashed in
full and must equal the committed source, and the identity it then has is committed
with the call's evidence. Provider calls compare the identity as before, and a last
identity comparison, not a full hash, precedes the commit.

This extends the residual accepted above for the bracketed binding across calls: a
same-user actor who rewrites the copy in place between two evidence calls and restores
every identity field is not detected by the later call. On Unix the kernel-set
status-change time makes that impossible for an unprivileged process; on Windows,
which has no such field, a rewrite that restores the modification time is caught only
by a full hash (a later call whose identity differs, or any mutating operation, which
still hashes). The recorded identity lives in the private session manifest, which
only the same user can write. Mutating and committing operations (ingest,
retranscription, candidates) keep their full-hash policy. Regression tests:
`evidence_calls_hash_the_copy_only_when_its_identity_is_new_or_changed` (unit, with the
test-only hash counter) and the engine's
`a_modified_source_copy_is_an_integrity_failure_and_nothing_is_committed`.
