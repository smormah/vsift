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
