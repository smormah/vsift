# ADR 0028: Online sources: direct HTTPS links and public video pages

- Status: **Proposed, 2026-10-21.** The design was sent to the supervisor on 2026-10-21 and
  given the go the same day, with the maintainer's decisions on the yt-dlp pin, the update
  path and joining separate audio and video, and the supervisor's on the rest (recorded below).
  It becomes Accepted with the merge of the first slice's pull request, after the `0.2.3` cut,
  for `0.3.0`.
- Date: 2026-10-21
- Tracking: [#406](https://github.com/smormah/vsift/issues/406) and its slices
  [#407](https://github.com/smormah/vsift/issues/407) (direct HTTPS links, built in) and
  [#409](https://github.com/smormah/vsift/issues/409) (public pages through an optional,
  managed yt-dlp). [#408](https://github.com/smormah/vsift/issues/408) (HLS and DASH) and
  [#410](https://github.com/smormah/vsift/issues/410) (live streams) are not in `0.3.0`.
- Amends: [ADR 0012](0012-p04-source-media-profile.md) (URLs stay refused at the media
  profile; acquisition happens before it, into the session), the R1 plan's exclusions
  ([ADR 0011](0011-r1-industrial-capability-expansion.md): acquisition is explicit, never
  automatic; no logins, no cookies, no authenticated sites), [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md)
  (two optional managed components, a core and optional split of the plan and the
  transaction), known limit L-046.

## Context

People are often sent a link to a video, not a file. An agent with a shell fetches it itself
today, with no bound, no record of what it fetched and no check of where the link points.
#406 (decided on 2026-10-15) makes online sources opt-in and offers a bounded, recorded
acquisition step: direct HTTPS links built in, platform pages through an optional yt-dlp that
the user adds. This ADR fixes the design of both slices for `0.3.0`.

## Decision

### 1. One command

`vsift ingest --url <https-url> [--transcript <file> ...]` (exclusive with the path). `--url -`
reads the link from standard input, so a signed link is never in the process list or the
shell history. It ends exactly where `ingest` ends: the result is the `ingest` result with an
additive `origin`, and every later command is unchanged and works offline. Only this option
uses the network; a link found in a transcript, a caption, a title or a frame is never
fetched. Worker job requests take no URL (the strict worker is loopback-only); `--url` with
`--host-isolation strict-linux` is refused before anything is fetched.

### 2. Placement: inside the session, from the first byte

The session is registered and initialised first, then the body is streamed straight into the
session's own source slot (`sessions/<ses>/artifacts/source-<op>.media`: created new, never
following a link, private, hashed as bytes arrive). A complete body is the snapshot, with no
second copy (#325, L-140). Any failure, cap or cancellation goes through the existing
abandonment of a failed open (#277): no partial file and no session remain. The downloaded
source is a session artifact like any copy: `session close`, expiry and `session clean`
remove it, and `session retain` includes it only with `--include-source`. Disposable stays
the default.

### 3. Direct HTTPS links (built in, #407)

- **Links:** HTTPS only; port 443 only (`0.3.0`); no user name or password in the link
  (refused, never sent); at most 2,048 characters.
- **Address policy where the connection is made:** a VSift `reqwest::dns::Resolve` resolves
  the name and returns only globally routable unicast addresses. A name with any address in a
  special range is refused whole: loopback, private (RFC 1918), link-local (including
  169.254.169.254 and the other metadata addresses), carrier-grade NAT, unspecified,
  broadcast, multicast, documentation, benchmarking, reserved, IPv6 unique-local and
  link-local, and the IPv4-mapped, IPv4-compatible, 6to4, Teredo and NAT64 forms of any of
  these. reqwest connects only to the addresses the resolver returned, so the address checked
  is the address connected to and DNS rebinding cannot get through. A host written as an IP
  address is never resolved, so the URL and redirect policy checks it directly, on every hop.
- **Redirects:** at most three, each re-checked (HTTPS, port, no credentials, address, and
  through a proxy an internal-looking name; amended 2026-10-22).
- **Proxies:** reqwest takes a proxy from `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` only.
  Through a proxy the proxy resolves the name and VSift cannot see the address. **Decided
  (supervisor, 2026-10-21):** VSift follows the proxy (organisations need it), refuses IP
  addresses in special ranges and internal-looking names (`localhost`, single-label names,
  `.local`, `.localhost`, `.internal`, `.lan`, `.home.arpa`, `.corp`), and records
  `address_check: "proxy_resolved"` in the origin; L-208 says what that leaves.
- **Caps (decided):** connect 15 s; no byte for 30 s stops the transfer; the whole transfer
  30 minutes by default (`--max-download-seconds`, at most 4 hours); 4 GiB by default
  (`--max-download-bytes`, at most the 20 GiB source maximum). Bytes are counted as they
  arrive, so a missing or false `Content-Length` cannot pass; a declared length over the cap
  is refused before the body; free space is checked for the declared length (or the cap)
  with the existing margin. *(Amended 2026-10-22: for a body of unknown length, 64 MiB at a
  time as it arrives, not the cap.)* The 4-hour duration is checked by the probe afterwards.
- **Response:** only a complete `200 OK`; a `206`, any `Content-Range` or a compressed body is
  refused (`Accept-Encoding: identity`). The server's content type is not trusted, but a text
  type stops at once as a web page. The first 12 bytes must be ISO base media (`ftyp`) or
  Matroska/WebM (EBML) magic, the only containers ADR 0012 admits, checked as soon as they
  arrive; anything else stops the transfer.
- **Headers and identity:** `GET` with `User-Agent: VSift/<version> link-ingest` (the product
  only), `Accept: */*`, `Accept-Encoding: identity`. No cookie store, no `Authorization`, no
  `Referer`. No telemetry; nothing else is contacted.
- **Failure code (decided):** a new code, `SOURCE_FETCH_FAILED` (exit 3), with typed reasons:
  `not_https`, `credentials_in_link`, `port_not_allowed`, `address_refused`, `tls`,
  `redirect_policy`, `http_status`, `proxy_auth`, `proxy_unusable`, `offline`, `stalled`,
  `deadline`, `size`, `partial`, `compressed`, `web_page`, `not_a_video`, `truncated`,
  `strict_isolation`. *(Amended 2026-10-22, see the review note below: `proxy_unusable` is
  added; `disabled_by_policy`, in the first draft, is reserved for `1.0` and added with its
  switch.)* `DOWNLOAD_FAILED` stays the setup code (its fixed message is about the
  reviewed publisher). Every remediation names the online step and its cause and says local
  files still work; none echoes the URL, the server's text or proxy credentials. `web_page`
  names the optional component (section 4) with `required_authority: "user_confirmation"`.

### 4. Public video pages (optional managed yt-dlp, #409)

This section is the whole design of #409, written so that each of its two halves can be built from it
alone. Since 2026-10-21 the **setup half** (4.1 to 4.5) is part of the cross-platform managed-install
work: the maintainer decided that FFmpeg, whisper.cpp, OCR and speaker components are installed on
Windows, macOS and Linux, and that work owns the component model, the archive readers, optional
components and the transaction. Each item below is marked **[managed-install]** when that work
provides it and **[acquisition]** when #409's acquisition half builds it. A first, stopped build of
the setup half is on draft pull request #503 (branch `v030/409-optional-components`, head
`1735e57f`). It is reference material only: the managed-install work may reuse or ignore it.

#### 4.1 The components and their pins [managed-install]

- **Two optional components:** `yt_dlp` and `js_runtime_deno`. Each is named by the identifier the
  JSON results already use, is never in a plan unless included, and is never needed for local
  investigation. Including `yt_dlp` includes `js_runtime_deno`, which is installed before it and
  removed with it.
- **Managed on Linux only (maintainer, 2026-10-21; unchanged by the cross-platform decision):** the
  managed yt-dlp and Deno are offered on the qualified Linux target, Ubuntu 24.04 x86-64. On Windows
  and macOS the plan lists `yt_dlp` as `unavailable`, and a user-supplied yt-dlp is the way to use
  it there (4.6).
- **yt-dlp pin:** stable `2026.08.19`, not a nightly. The file is `yt-dlp_linux` from
  `https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_linux`: 40,446,224 bytes,
  SHA-256 `58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a`. It is a raw-file
  artifact installed as one owner-executable file, `yt-dlp`. Catalogue version key
  `yt-dlp-2026.08.19-linux-x86_64`. Banner: the first line of
  `yt-dlp --ignore-config --no-plugin-dirs --version` is `2026.08.19`.
- **yt-dlp provenance:** the hosted review checks, at review time and never on a user's machine,
  that the pinned digest appears in the release's GPG-signed `SHA2-256SUMS` under its file name:
  - `SHA2-256SUMS`: 1,595 bytes, `a63701f30755cb4d9317950d69703e9d751d490cb1b7059e1bb501a353fe7dcb`;
  - its signature `SHA2-256SUMS.sig`: 566 bytes,
    `8d6f9a7c84c57222e3650c61699185878a3468308de702993ce4f20b5f9cf199`;
  - the key is `public.key` at the tag, Git blob `b3af31e9aa2476149045e56ebe82c4a9c41cf09c`,
    fingerprint `AC0CBBE6848D6A873464AF4E57CF65933B5A7581`.

  The same signed sums list the files a user may supply elsewhere:
  - `yt-dlp.exe`: 17,840,399 bytes,
    `66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a`;
  - `yt-dlp_macos`: 37,146,048 bytes,
    `0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202`.

  The tool is `tools/online_video_component_review.py` with its workflow
  `Online video components review`, on #503. Run 39535463204 (2026-10-21) passed: the signature
  verified and both banners were read.
- **Deno pin:** `2.9.7`, from
  `https://github.com/denoland/deno/releases/download/v2.9.7/deno-x86_64-unknown-linux-gnu.zip`.
  - The zip is 41,596,794 bytes, SHA-256
    `c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490`.
  - It holds exactly one entry, `deno`: deflated, 41,596,636 bytes compressed, mode `0100755`,
    CRC-32 `1c70df98`, 95,830,104 bytes, SHA-256
    `ce6a052beb97c2b92de67077e3f3924ba7c9661ede0d8dc2f66a116a3c841f21`.
  - The publisher's `deno-x86_64-unknown-linux-gnu.zip.sha256sum` (100 bytes,
    `bf6ee050dc421189520a9ef6dbf9f868cbbdb1e5eda1a28c37dad34e7962a520`) and
    `deno-x86_64-unknown-linux-gnu.sha256sum` (71 bytes,
    `d2318dc4a0f945d0955c2e820bb4e925366e043a2e1b4f14d6bb482c5f043ef3`) agree with both pins.
  - Deno publishes no signature, so the pinned digests are the anchor.
  - Catalogue version key `deno-2.9.7-x86_64-linux-gnu`. The archive bounds are 1 entry and
    95,830,104 expanded bytes. The banner prefix is `deno 2.9.7 `; the full line read by the review
    was `deno 2.9.7 (stable, release, x86_64-unknown-linux-gnu)`.
- **The zip reader:** Deno needs a bounded reader for a zip of one regular file, with its own fuzz
  target. #503 has one (`one_entry_zip.rs`). Its policy:
  - the whole file must be accounted for: a local header at offset 0, its data, an optional data
    descriptor, one central record, and an end record with no comment that ends the file;
  - the entry must be stored or deflated, unencrypted, not Zip64, on disk 0, a regular file, and
    named exactly as reviewed;
  - its expanded bytes are counted as they are produced, and the entry is accepted only when its
    size, CRC-32 and SHA-256 equal the review and the deflate stream used exactly the declared
    compressed bytes.

  If the managed-install work brings a general zip reader for Windows and macOS archives, Deno uses
  that instead.
- **Licence:** yt-dlp's source is under the Unlicense, but its PyInstaller executables include
  GPLv3+ code, so the file VSift fetches is GPL-3.0-or-later as a combined work. VSift never
  redistributes it: the publisher's file is fetched onto the user's machine at the user's request
  and run as a separate process, with no linking, so VSift's MIT/Apache licence is unaffected.
  Deno is MIT.

  The plan, `setup check` and the third-party notices say so, with the notice and source URLs:
  - yt-dlp: `README.md` and the tree at the tag;
  - Deno: `LICENSE.md` and the tree at `v2.9.7`.

  Downloading from a site is subject to its terms, which is the user's responsibility; the plan and
  the documentation say so.
- **The update path (maintainer):** pins move with VSift releases in `0.3.0`; a signed component
  manifest refreshed between releases is `1.0`'s. A re-pin repeats the hosted review and the phrase
  check (4.8).

#### 4.2 The plan [managed-install]

- `setup plan --profile desktop --include yt_dlp` (repeatable; it takes the component names). The
  inclusion is part of the plan and its digest, so `setup install --accept-plan` installs exactly
  what was shown. A plan that includes nothing is digested exactly as before.
- **Every desktop plan lists each optional component** in `optional_components`. Each entry has:
  - `component` and `status` (`included`, `not_included` or `unavailable`);
  - `installs` (`js_runtime_deno`, `yt_dlp`) and the `download_bytes` (82,043,018 together);
  - its `licences` and a `terms_notice`;
  - a one-sentence `summary` an agent can relay, for example: "Not included: yt-dlp (optional,
    about 83 MB to download; GPL-3.0-or-later, with the MIT-licensed Deno runtime) fetches videos
    from YouTube and other sites when you give VSift a link. Downloading is subject to each site's
    terms.";
  - the `add_command` as an executable and argument array;
  - and, when included, its exact actions.

  A worker plan lists only what it includes.
- **`operator_decision_required`** is a string present whenever the plan offers or includes an
  optional component. It tells whoever runs setup to:
  - tell the operator what the component does, its licence, its size and that each site's terms
    apply, then ask before accepting the plan;
  - include it only after a yes;
  - never accept a plan with it without a yes, and never leave it unmentioned.

  It is in the plan, so an agent with no skill relays it too. The human output shows the optional
  part near the top, not as a footnote.
- `setup install --artifact-dir` installs the optional files too when the plan includes them.

#### 4.3 The transaction [managed-install]

- **The order:** every core component is installed first, then the optional components, staged and
  smoked together, then activated runtime first.
- **A core failure** stays a failure result exactly as today, with the optional part `blocked`.
- **An optional failure** never blocks or fails a core component. When the core is ready, the
  result is a success (exit 0) whatever happened to the optional part, with:
  - `core: "ready"`;
  - `optional_components`, each with its typed reason (the download reasons `tls`,
    `redirect_policy`, `http_status`, `proxy_auth`, `offline`, `size`, and the smoke reasons);
  - an `online_video` object (`installed` or `not_installed`, the failing component, its reason and
    code, and fixed advice that says local investigation is not affected);
  - an envelope warning, `online_video_not_installed`.

  The three members are absent when the plan includes no optional component, so earlier results keep
  their shape.
- **Open decision for the maintainer:** the exit status when only the optional part fails. The
  recommendation is success, with the data and the warning, so a script that needs online video
  checks `online_video`.

#### 4.4 The smoke [managed-install]

- `deno --version` and `yt-dlp --ignore-config --no-plugin-dirs --version` must print their
  reviewed banners. They run with:
  - an otherwise empty environment;
  - `TMPDIR`, `TEMP` and `TMP` set to the stage's private smoke folder (the one-file yt-dlp unpacks
    itself there);
  - `YTDLP_NO_PLUGINS=1` for yt-dlp, and `DENO_NO_UPDATE_CHECK=1` and `NO_COLOR=1` for Deno.
- The smoke folder must be empty afterwards, so a one-file build that leaves what it unpacked fails
  the recheck (`unexpected_extra_file`).
- The hosted review started both executables this way once. Whether PyInstaller always removes its
  folder on the qualified Linux target is checked by the opt-in hosted run of the real component
  (4.9).

#### 4.5 Check, list, remove, rollback [managed-install]

- **`setup check`** gains `online_video`, apart from the local-video `status`, which it never
  changes:
  - `installed`, with the versions;
  - `not_installed`, with the add command on the desktop profile;
  - `unavailable` off the managed target.

  A superseded optional version is reported in `managed_updates` like any other.
- **`setup list`** lists an optional component after the core only when the store holds something
  of it.
- **`setup remove`, `setup rollback` and `setup repair`** take the two names like the others, and
  the managed store recognises their version folders.

#### 4.6 A yt-dlp the user supplies [acquisition]

- `setup configure yt_dlp --executable <absolute-path>` registers a yt-dlp. With it, the user may
  register a JavaScript runtime: `setup configure js_runtime --executable <path>`, Deno only in
  `0.3.0`.
- **On Windows and macOS** this is the way to use pages. On Linux it takes precedence over the
  managed one, like every configured tool.
- **Reporting:** it is reported as user-managed, with its version line and SHA-256, and its lower
  trust: keeping it current is the user's job. A managed yt-dlp is never run with a user's runtime,
  and a user's yt-dlp never with the managed Deno, so the pinned pair is only ever run together.
- **Same rules:** VSift runs it with the same fixed invocation and checks its output the same way.
  Without a runtime, YouTube may refuse with `js_runtime_missing`.

#### 4.7 Acquisition [acquisition]

- **Routing:** `ingest --url` applies section 3's checks first.
  - When the link answers with a web page (`web_page`) and a yt-dlp is resolved (configured, then
    managed), the link goes to yt-dlp.
  - Without one, the refusal is `web_page`, whose remediation names the optional component and gives
    the add command with `required_authority: "user_confirmation"` (on Windows and macOS: the
    `setup configure yt_dlp` path).
  - VSift keeps no list of sites. A link to a host written as an address, or to an internal name,
    is refused before yt-dlp is started, by section 3's rules.
- **The environment:** the explicit executable and an argument array, with no shell. The
  environment is empty except:
  - `TMPDIR`, `TEMP` and `TMP`, set to a private folder in the session's work directory (removed
    with the session);
  - `YTDLP_NO_PLUGINS=1` and `HOME` set to that same folder, so no user configuration is found;
  - the proxy variables VSift itself honours (`HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`), passed
    through the environment and never on the command line, where other local users could read
    them.
- **The fixed arguments:**
  - `--ignore-config --no-plugin-dirs --no-update --no-remote-components`;
  - `--no-cookies --no-cookies-from-browser --no-playlist --no-batch-file --max-downloads 1`;
  - `--abort-on-error --no-exec --restrict-filenames --no-mtime --no-part --no-continue`;
  - `--downloader native --no-check-formats --socket-timeout 30`;
  - `--js-runtimes deno:<runtime path>`, after `--no-js-runtimes` so a runtime on `PATH` is never
    used;
  - `-P <work folder> -o <fixed name>.%(ext)s`;
  - the link after `--`.

  Never `--exec`, `--netrc`, `--netrc-cmd`, `--username`, `--password`, `--cookies`,
  `--use-postprocessor`, `--downloader-args`, `--write-link`, `--ffmpeg-location` or
  `--extractor-args` from input. `--use-extractors` is not used in `0.3.0`: there is no qualified
  site list to name.
- **The format and live filter:** the format selection is `bv*[protocol=https][ext=mp4]+ba[protocol=https][ext=m4a]/b[protocol=https][ext=mp4]/bv*[protocol=https][ext=webm]+ba[protocol=https][ext=webm]/b[protocol=https][ext=webm]`,
  with `--merge-output-format` never given, because VSift joins the streams (below). The
  `--match-filters` value is `!is_live & !was_live & live_status!=is_upcoming & live_status!=post_live`.
  Every flag name and filter field is rechecked against the pinned tag's `--help`, by the hosted
  phrase check.
- **Phase 1, the metadata:** `--simulate --dump-single-json`. VSift stops the process tree when
  standard output passes 8 MiB, after 120 s, or on cancellation. The JSON is parsed strictly (the
  fields VSift reads are typed; the rest is ignored), and this parser gets its own fuzz target,
  `ytdlp_info_json`. VSift refuses:
  - a `_type` other than `video`, and any `entries` (playlists, channels);
  - `is_live`, `was_live` and `live_status` other than `not_live`;
  - an `availability` of `private`, `needs_auth`, `premium_only` or `subscriber_only`;
  - a `duration` over 4 hours, or none;
  - a selected format whose `protocol` is not `https`, or whose `ext` is not `mp4`, `m4a` or
    `webm`;
  - a `filesize` or `filesize_approx` over the cap.
- **Phase 2, the download:** the same arguments without `--simulate`, into the work folder, under
  the caps of section 3: bytes counted on disk every 200 ms, the 30-minute default deadline, and a
  stall of 30 s with no growth. When a cap is passed, VSift stops the process tree (SEC-04).

  Afterwards the work folder must hold exactly the expected regular files, one, or two when the
  format was a joined pair:
  - not a link, and nothing outside the folder;
  - names with the fixed stem;
  - each with admitted magic (`ftyp`, or EBML).

  Anything else is refused and the folder removed. One file is moved into the session's source slot
  with no copy, and from there it is an ordinary source (the snapshot, the probe, ADR 0012's media
  profile).
- **Joining separate audio and video (maintainer):** both files are checked as above, then joined
  into the session's source slot by the pinned FFmpeg as a stream copy only:

  `ffmpeg -nostdin -hide_banner -loglevel error -protocol_whitelist file -f <mov|matroska> -i <video> -f <mov|matroska> -i <audio> -map 0:v:0 -map 1:a:0 -c copy -map_metadata -1 -f <mp4|matroska> <slot>`

  - The demuxer comes from the magic, `mp4` output for `ftyp` inputs and `matroska` for EBML ones.
    No filter, no re-encode, no other input or protocol.
  - The join runs under the same time and size caps. The slot is written only by FFmpeg, and is
    then hashed like any source.
  - The threat review records that FFmpeg demuxes two hostile inputs, the same exposure as any
    ingest. It also records that this is the first use of FFmpeg as a muxer, so the `mp4` and
    `matroska` muxers join the scan scope of every reading.
- **The page origin (section 5):**
  - kind `platform_page`, `provider` (yt-dlp's `extractor_key`, checked to be 1 to 64 letters and
    digits);
  - `video_id` in clear only when `availability` is `public`, otherwise only its SHA-256
    (`video_id_sha256`);
  - `link_sha256`, `yt_dlp` (version and catalogue version, or `user_managed` with its SHA-256),
    `js_runtime` (the same);
  - `retrieved_at`, `bytes`, `source_id`, `joined` (`single_file` or `audio_video`).

  Never the title, uploader, description, upload date, the selected format's URL, or any header.

#### 4.8 Typed refusals and the phrase set [acquisition]

yt-dlp reports most causes only as text on standard error, and its exit status does not tell them
apart. VSift reads its standard error, at most 64 KiB, only to choose a reason; the text is never
echoed, stored or logged. The phrases below are the candidate set. Each must be found, verbatim, in
the pinned tag's source archive by the hosted phrase check before it is used, and the check fails
when a re-pin changes one.

| Reason | Candidate phrases (case-sensitive substrings) |
| --- | --- |
| `login_required` | `This video requires login`; `--cookies-from-browser or --cookies for the authentication`; `Join this channel to get access`; `members-only content` |
| `age_restricted` | `Sign in to confirm your age`; `age-restricted` |
| `bot_check` | `Sign in to confirm you’re not a bot` (the typographic apostrophe, as yt-dlp prints it); `confirm you're not a bot` |
| `drm` | `This video is DRM protected`; `DRM protected` |
| `region_blocked` | `not available in your country`; `geo restriction`; `The uploader has not made this video available` |
| `private` | `Private video`; `This video is private` |
| `removed` | `Video unavailable`; `This video has been removed`; `account associated with this video has been terminated` |
| `unsupported_site` | `Unsupported URL` |
| `extractor_broken` | `Unable to extract`; `please report this issue on`; `Requested format is not available` |
| `js_runtime_missing` | `No supported JavaScript runtime could be found`; `n challenge solving failed`; `JS runtime` |
| `proxy_auth` | `407 Proxy Authentication Required` |
| `network` | `Unable to download webpage`; `Failed to resolve`; `Connection refused`; `timed out`; `HTTP Error 403` |

`age_restricted` is checked before `login_required`, because yt-dlp's age message asks to sign in.

VSift's own checks give their own reasons: `playlist`, `live`, `too_long`, `not_https_format`,
`not_a_video`, `size`, `deadline`, `stalled`, `unexpected_output`. Anything else is
`online_step_failed`.

Every reason is a `SOURCE_FETCH_FAILED` reason (exit 3), added to the typed reasons of section 3.
Its remediation:
- names the online step and its cause, and says local files still work;
- for `extractor_broken`, says to plan again for a newer reviewed yt-dlp (managed) or update it
  (user-supplied), and not to report it as a VSift defect;
- for `js_runtime_missing` with a user-supplied yt-dlp, says to configure the runtime;
- never echoes the URL, yt-dlp's text, cookies or proxy credentials.

#### 4.9 The fake yt-dlp and the tests [acquisition]

- **The fake yt-dlp:** a standard-library test binary like `vsift-smoke-fixture`, staged under the
  name `yt-dlp`. It writes its argument list and its environment (names, and the values of the
  variables VSift sets) to a file in its temporary folder, which the test reads and then removes. It
  then replays a scenario the test chooses through a file next to the executable, never through an
  argument or a variable VSift would have to pass. The scenarios:
  - recorded phase-1 JSON;
  - one file, and two streams to join;
  - an output outside the work folder, a link, and two files where one was expected;
  - a wrong container, an oversize file, a hung process, a growing file that stalls;
  - and one per phrase of 4.8 on standard error, with an unrecognised error as well.

  The tests check the exact argument list and environment, for the managed and the user-supplied
  yt-dlp; each typed reason and its remediation; and that no sentinel token, cookie or proxy
  credential appears in any output, record or log.
- **Opt-in hosted runs** against the real managed yt-dlp:
  - a planted `yt-dlp.conf` next to the executable, in the work folder and in `HOME`, and a planted
    plugin directory, are all ignored;
  - the TMPDIR folder is empty after each run;
  - RM06's Internet Archive page is fetched through the managed yt-dlp;
  - optionally, one NASA video on YouTube checks the typed refusal of a bot wall.
- **The setup half's tests [managed-install]:**
  - the plan with and without `--include`, its digest and its offer;
  - the transaction's order, and its optional failure leaving a success;
  - the zip reader and its fuzz target;
  - the smoke's environment, and its recheck failing on a leftover file;
  - on the managed target, fault tests in the style of D-07, each with core stand-ins already
    current and the optional download through a proxy: one that answers `407`, one that blocks the
    host, one that cannot be resolved, and a closed port. In each, the result is a success with
    `online_video: not_installed` and the typed reason, the sentinel credential appears in no
    output, and `setup check` reports the local-video profile unchanged.

  #503 has unit and contract versions of most of these.

### 5. Privacy and provenance

The raw URL is never stored, printed or logged: it can carry signed tokens, in its query or in
its path. The session stores an `origin` record (an optional lifecycle member, as for time
anchors):

- for a direct link: kind `direct_https`, `link_sha256` (SHA-256 of the parsed, normalised
  link as given, query included, so the same link is recognisable but not recoverable),
  `requested_host`, `final_host`, `redirects`, `address_check` (`direct` or
  `proxy_resolved`), `retrieved_at` (UTC), `bytes` and `source_id` (checked equal to the
  session's source identity);
- for a page: kind `platform_page`, `provider` (yt-dlp's extractor key), `video_id` **in clear
  only when the platform reports the video public**; an unlisted video's id is a capability
  token, so only its SHA-256 is kept (decided); `link_sha256`, the yt-dlp version and its
  catalogue entry (or "user-managed" and its executable's digest), the runtime's version,
  `retrieved_at`, `bytes`, `source_id`, and whether the source was joined from two streams.

A page's title, uploader, description and upload date are not stored: they are untrusted text
and other people's personal data, and **an upload date is not a recording time**, so it is
never a time anchor (ADR 0029). The origin goes into a retained bundle only through `session
retain`.

### 6. Tests run without the network

- A hand-written local HTTPS server (as the setup tests' server, with `native-tls`) presents
  a committed test CA and leaf for `video.example.test`, trusted only through the
  development-only test route (built on the existing `install-test-hooks` feature, refused in
  release builds and by the governance check), which also injects a test resolver and the
  loopback exception for the test route only. Every acceptance case of #407 is covered, and
  each leaves no partial file and no session. A production-build test proves loopback and
  private addresses are refused with no hook. Redaction tests put a sentinel token in a
  query, a fragment and a path and find it in no output, record, bundle or log.
- #409's tests, the fake yt-dlp among them, are in section 4.9.
- **Live checks** are a manual hosted workflow against NASA public-domain items only: RM09
  on `images-assets.nasa.gov` (direct), RM06 on the Internet Archive (direct, through its
  redirects to storage nodes), RM06's Internet Archive page through the managed yt-dlp, and
  optionally one NASA video on YouTube, where a bot wall on a hosted runner is expected and
  checks the typed refusal. Wikimedia refuses neutral user agents and is never a target.
  Nothing is downloaded to a development machine.

## Delivery

Three parts, each standing on its own:

1. **#407's slice:** sections 1 to 3, section 5 for direct links, and section 6's first part (the command,
   the address policy, the origin, `SOURCE_FETCH_FAILED` and its tests). This is pull request #502.
2. **#409's setup half:** sections 4.1 to 4.5. Since 2026-10-21 it is delivered by the cross-platform
   managed-install work, which owns the component model, the zip reader, optional components and the
   transaction (maintainer). Draft #503 is a stopped first build, kept for reference only.
3. **#409's acquisition half:** sections 4.6 to 4.9 and the page half of section 5 (the invocation, the
   refusals and their phrases, joining, the page origin and the fake yt-dlp). It needs the setup half's
   components, or a user-supplied yt-dlp, and is briefed to a fresh agent when `main` opens for `0.3.0`.

This ADR is written once for all three; each pull request says which sections it delivers.

*Implementation note, 2026-10-21 (#407's pull request):* sections 1 to 3, the direct-link half of section 5
and the first part of section 6 are built: `vsift ingest --url` and `Engine::ingest_link`, the domain values
(`SourceLink`, `DownloadLimits`, `SourceFetchReason`, `SourceOrigin`), the fetch in
`crates/vsift-infrastructure/src/link_acquisition.rs` (the address policy, the resolver, the proxy rule, the
response and body checks) writing into the session's source slot (`SourceSnapshot::stage_link`), the stored
`origin` lifecycle member, `origin` in `ingest` and `session status`, `SOURCE_FETCH_FAILED` with
`error.reason`. The development-only link test route reuses the `install-test-hooks` feature rather than a new
one, so the governance check's existing rule covers it. The skill changes in one place only: its handoff
schema's gap codes gain `SOURCE_FETCH_FAILED`, with the matching row of the vocabulary table in
`references/handoff.md`, because the skill's contract test requires the list to equal the failure codes. That
voids the trial freeze, which is rewritten at this pull request's rebase (the supervisor's decision). Its "do
not use for streaming URLs" is still true of pages and streams, and its wording changes once, at the `0.3.0`
freeze, with #409's.

*Review note, 2026-10-22 (RV-3, before #502's merge):* a read-only review of #502 found one high, three medium and
five low findings; the pull request changes these before it merges, and this ADR's decisions otherwise stand.

- **Internal names through a proxy, on every hop.** Through a proxy the redirect policy now refuses an internal-looking
  name on each hop, as the link's own host is refused (section 3, "Redirects" and "Proxies"): before, only the first
  URL was checked, and a public link that redirected to an internal name was followed through the proxy.
- **Three redirects means three.** The policy compared with `>=` and refused the third redirect; it now follows
  three and refuses a fourth.
- **Free space for a body of unknown length** is checked 64 MiB at a time as it arrives (the store's own rule and
  margin each time), not for the whole download limit before the first byte, which refused a small video sent without
  a length on a volume with less free space than the limit. A failed write still reports a shortage, as for a copy.
- **A proxy value that is set is used or refused, never ignored.** A proxy value is read as curl reads it (a URL, or
  `host:port` for an HTTP proxy). Any other value, or a scheme other than `http` or `https`, is the new reason
  `proxy_unusable`, whose remediation names the variables and never the value. Before, an unparsable value sent the
  fetch direct, past the proxy the user set.
- **`disabled_by_policy` is reserved for `1.0`** (the planner's decision of 2026-10-22). Nothing produced it, and a
  published v1 reason cannot be removed; it is added, additively, with the organisation-wide switch that returns it
  (see Consequences), which moves to `1.0`.
- **Documented:** `SOURCE_FETCH_FAILED` is `retryable: false` as a code, and the retry of a transient reason is the
  caller's (`cli-v1.md`); the `NO_PROXY` forms read (troubleshooting); cancellation and the deadline are observed while
  the fetch waits for the network, not during a chunk's write or the final flush; the CLI contract cases use a
  `.invalid` host, so a release-profile test run reaches no server either.

*Maintainer's answers, 2026-10-23 (on #502's open questions):*

- **The typed, optional `error.reason` is accepted.** It is an additive member of the v1 error object, present for
  `SOURCE_FETCH_FAILED` only; readers that predate it see an unknown code with its fixed remediation.
- **The link test route on `install-test-hooks` is accepted.** The development-only route that lets the tests fetch
  from a local HTTPS server stays on the existing feature, covered by the governance check's rule, and is not in
  release builds.
- **The live check runs by hand.** The hosted `Online links live` workflow (manual dispatch) runs at each
  release cut and in the monthly full-cost round, with no schedule of its own; nothing runs it unprompted.
- **The machine-wide `--url` switch comes in `1.0`.** `disabled_by_policy` is already left out of `0.3.0`
  (review RV-3, above) and is added, additively, with that switch.

*Implementation note, 2026-11-01 (#409's setup half, draft pull request on `v030/409-setup`):* sections 4.1 to 4.5 were
in place from the managed-install work (#504) before this pull request, and this one checked them against the code
rather than rebuilding them. What #504 built: the `online_links` group with the pins of 4.1, `setup plan --include`
and the plan's `optional_components` and `operator_decision_required` (4.2), the split transaction (4.3), the smoke
with the fixed banners, environment and the folder that must stay empty (4.4), and `setup check`, `setup list`,
`setup remove`, `setup rollback` and `setup repair` for the two components (4.5). Two differences from the text above
stand. The members 4.3 names (`online_video`, `online_video_not_installed`) are the group-generic `optional_groups`
and the warning `optional_component_not_installed` of ADR 0032, and the partial result exits 0 as recommended. And
`setup check`'s `online_video` of 4.5 is the `online_links` entry of `optional_components`.

This pull request adds what was left, plus 4.6's registration (the acquisition half runs what is registered):

- **The spelling in 4.6 and 4.7 is `yt-dlp` and `js-runtime`** (the planner, 2026-10-31): `vsift setup configure
  yt-dlp --executable <absolute-path>` and `vsift setup configure js-runtime --executable <absolute-path>`, beside
  `ocr-runner` in one list of values. All help and remediation text uses these spellings, including the
  `web_page` remediation on Windows and macOS. `yt_dlp` is accepted as a hidden alias, because `--include
  online_links` and the JSON results use the underscore; the runtime has no alias, since an alias on a stable command
  line stays for good. The JSON identifiers are `yt_dlp` and `js_runtime`. Each is stored in a file of its own
  (`yt-dlp-v1.json`, `js-runtime-v1.json`), as the OCR runner is, so an earlier release reads its own configuration
  unchanged. `configure` records a canonical path and runs nothing, like the others.
- **Resolution is by pair, not by tool.** "Configured first, then managed" applies to yt-dlp, and the runtime follows
  yt-dlp's source: a managed yt-dlp runs with the managed Deno only, a user's with the user's runtime or with none
  (it then passes `--no-js-runtimes` alone, and `js_runtime_missing` is the honest outcome), never the managed Deno. So
  a registered runtime without a registered yt-dlp is unused. `configure` still accepts it, because the order must not
  matter, and `setup check` says it is unused (`unused_components`, and a sentence in `next_step`) rather than
  reporting it in use.
- **`setup check` runs the registered files as untrusted programs.** A yt-dlp gets the fixed `--ignore-config
  --no-plugin-dirs --version`, an otherwise empty environment (on Windows with `SystemRoot`, below), `YTDLP_NO_PLUGINS=1` and a private temporary and home
  folder in the per-user verification area (removed afterwards); Deno gets `--version`, `DENO_NO_UPDATE_CHECK=1` and
  `NO_COLOR=1`. Nothing fetches a URL. The version line is bounded (64 printable ASCII bytes, the first line only) and
  must have the shape of what the tool prints: a date version for yt-dlp, `deno ` first for the runtime, so a runtime
  that is not Deno is not healthy (Deno only in `0.3.0`). The SHA-256 read is bounded at 512 MiB. Output is bounded at
  4 KiB, and the run at 30 s at most: the files are identified one after the other within what is left of the deadline all tool checks share (`probe_timeout`, 30 s by default and 60 s at most with `--timeout-seconds`), spent once by the dependency probes first, and with nothing left a file is not read or started and the reason is `deadline`. What is reported is `source: "user_managed"`, the line and the digest, with the lower trust:
  nothing reviewed the file, and keeping it current is the user's job. A file that cannot be identified makes the
  group `not_installed` with the reason in words and no `add_command`.
- **The plan's `unavailable` sentence** on Windows and macOS names `setup configure yt-dlp`.
- **The environment of a registered tool.** The process supervisor starts it with an empty environment plus the
  private folder (`TMPDIR`, `TEMP`, `TMP` and `HOME` alike) and the fixed variables above. On Windows the one extra
  variable is `SystemRoot`, copied from VSift's own environment: a one-file Python build there cannot start without
  it (Python's random-number start-up needs it), and it names the system folder, not a person. It was added before any
  hosted run could show whether it is needed; the Windows job below runs with it, and the acquisition half runs yt-dlp in the same
  environment. If a hosted run ever shows it unnecessary, remove it here and in the acquisition half together.
- **Tests:** the two registration records; the CLI contract for registering, the hidden alias and the refused
  `js_runtime`; the unused runtime and a runtime that is not Deno (using the build's own binary as the stand-in, whose
  version line does not begin `deno `); a yt-dlp that fails its version request; the identification itself against
  the smoke's stand-in (a good answer, a wrong shape, a failure, a flood, and a hang that ends at the time it was
  given, with nothing started when none is left); the door, which looks through an unused runtime in the interactive
  and unattended paths; the contract rows and their schema; the lifecycle commands on the two components; and three
  opt-in hosted jobs in `P13 managed smoke`, dispatched together once: `managed-install-online-links` (Ubuntu: the real
  yt-dlp and Deno are installed with `--include online_links`, smoked, listed and one removed),
  `user-yt-dlp-windows` and `user-yt-dlp-macos` (the publisher's pinned `yt-dlp.exe` and `yt-dlp_macos` of 2026.08.19 are
  fetched by the runner, size-checked, registered with `setup configure yt-dlp`, and `setup check` must report them as
  `user_managed` with that version and the pinned digest; no page is fetched). The jobs are dispatch-only and
  skip the published mode and the weekly schedule, and the two older install commands now name their test, because an
  unnamed `--ignored --exact` run would run every ignored test in the file.
- **Not in this pull request:** the acquisition (`ingest --url` of a page), which uses what is registered; a managed or
  pinned Deno for Windows and macOS (this ADR pins none, and a pin needs its own review), so the Windows and macOS jobs
  register a yt-dlp only; `setup install --artifact-dir` for the optional files is the shared import path of the core
  and is not separately tested for them.
*Implementation note, 2026-11-01 (#409's acquisition pull request, W4b; the planner's ruling of the same day):*
reading the pinned yt-dlp's source showed section 4.7 could not work as written, so its arguments change. This was
read in the source at tag `2026.08.19`, never run; the hosted check confirms it, and if real yt-dlp disagrees the
evidence goes back to the planner.

- **Why.** `--max-downloads 1` with `--simulate --dump-single-json` ends a run with exit 101 and no JSON, because the
  limit is raised before the dump. Its exit 101 would also replace a failed download's exit 1. `--abort-on-error` stops
  a merged pair before either stream downloads when no FFmpeg is visible, and VSift keeps FFmpeg from yt-dlp on
  purpose. `live_status!=is_upcoming` is false for a site that reports no status. `--dump-single-json` implies
  simulate, so phase 2 cannot keep it.
- **Every run:** `--ignore-config --no-plugin-dirs --no-update --no-remote-components --no-cookies
  --no-cookies-from-browser --no-playlist --no-batch-file --playlist-items 1 --abort-on-error --no-exec
  --restrict-filenames --no-mtime --no-part --no-continue --downloader native --no-check-formats --socket-timeout 30
  --no-progress --fixup never --match-filters "!is_live & !was_live & live_status!=?is_upcoming &
  live_status!=?post_live" --output "vsift-page.%(format_id)s.%(ext)s" --no-js-runtimes [--js-runtimes
  deno:<runtime path>] --paths <output folder>`.
- **Phase 1** adds section 4.7's format selection, `--simulate --dump-single-json`, then `-- <link>`.
- **Phase 2** adds `--format <selection>`, then `-- <link>`. The selection is exactly the ids phase 1 checked:
  `<id>[protocol=https]` for one file, `<video id>[protocol=https],<audio id>[protocol=https]` for a pair. The comma
  makes two separate downloads, so no merge is considered, and the filter holds HTTPS on the page as yt-dlp reads it
  again. Ids come from `requested_formats`, or the top-level `format_id` for one file, and must match
  `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`, otherwise `unexpected_output`.
- **`--max-downloads` is not used.** What bounds a run is the single entry, the exact ids, the byte caps and the
  exact folder check. **Exit 0 is the only success in either phase:** with `--no-part` a failed download leaves a
  good-looking file under its final name, so the folder's check cannot stand in for the status.
- **Phase 1's rule for a missing field:** an absent `live_status` passes; any present value other than `not_live`
  is refused. The filter's `!=?` says the same.
- **`--fixup never`:** on Windows a user's `yt-dlp.exe` could find an unverified `ffmpeg.exe` beside it with `PATH`
  empty, and no FFmpeg may touch these files but the verified one.
- **The folder check** expects exactly `vsift-page.<id>.<ext>` for each format.
- **Environment:** `DENO_NO_UPDATE_CHECK=1` and `NO_COLOR=1` when a runtime is passed, as `setup check` runs Deno; on
  Windows `SystemRoot`, as W4a's identification does.
- **The join:** `-protocol_whitelist file` on each input, because it is an input option and section 4.7's line covers
  only the first; MOV inputs also get `-enable_drefs 0 -use_absolute_path 0`, as every other read does.
- **A new reason, `media_tools_missing`:** a pair with no verified FFmpeg, refused before the download.
- **The stall:** 120 s to the first byte, because phase 2 reads the page again, then 30 s without growth in the total
  across both streams, with no second allowance.
- **Links:** a host written as an address, and an internal name, are refused always, not only through a proxy,
  because yt-dlp connects with no address policy.
- **The page origin:** `catalogue_version` alone for a managed tool (the key names the exact version), `source_id`
  not repeated (the session's result has it, as for a direct link), and `address_check: "proxy_resolved"` only when a
  proxy was handed over: `direct` would claim a check yt-dlp's connections never had.
- **What the record cannot see (the planner's review of #561).** With no `--proxy`, yt-dlp calls
  `urllib.request.getproxies()` (`YoutubeDL.py:4215`), which falls back to the Windows registry or the macOS system
  settings when the environment has none, so `through_proxy` can be false for a proxied fetch by a user's yt-dlp. The
  link is on yt-dlp's command line, where other local users can read it while it runs; section 5's "never stored,
  printed or logged" is about VSift's own output. Both are stated in L-209 and the threat model's table.
- **A registered tool is not pinned by digest (the planner's decision on #561).** A person updates their own yt-dlp
  as a matter of course, so a pin would report every ordinary update as a change. Each fetch identifies the file
  again, and each origin records the digest read when it was identified for that fetch; nothing holds the file
  between that read and the run, and it is the user's own file. L-247 stays open.
- **The live checks (the planner's review of #561).** `Online links live` gains two dispatch-only parts, which never
  run on a pull request. RM06's page goes through the pinned user-supplied yt-dlp on Windows and macOS, with no
  runtime, which is also the only test of the engine's path through a registered yt-dlp. With the `youtube` input, and
  after those pass, one NASA video on YouTube runs through the managed pair, once for this pull request and at each
  re-pin: the typed `bot_check` passes, and so does a success with `joined: audio_video`; anything else fails and goes
  to the planner with the evidence.
- **The fake yt-dlp** keeps its record beside its executable, and the test's folder takes it away: its temporary
  folder goes with the session, so a record there could not be read. It also plays `FFmpeg` for the join.
- **The phrase table (section 4.8) is split.** Hosted run 41269645907 found seven candidates missing from the source
  archive, so "each must be found verbatim" could not hold for them. This amends that rule for six phrases only,
  and every other phrase stays under it. A phrase only chooses a reason and its remediation; it never accepts anything,
  and a site that rewords one falls to a broader reason. `YTDLP_PHRASES` are checked as section 4.8 says.
  `SITE_PHRASES` are exactly these six, with where they come from (line numbers at the pinned tag, read in the source):
  - `Sign in to confirm you’re not a bot`, `confirm you're not a bot`, `Join this channel to get access`,
    `members-only content` and `account associated with this video has been terminated`: YouTube's own `reason` or
    `subreason`, read from the player response and passed on at `extractor/youtube/_video.py:4053-4076`;
  - `407 Proxy Authentication Required`: the proxy's status line in the HTTP library's error, wrapped as the cause at
    `networking/_requests.py:344-345` and `networking/_urllib.py:434`.

  Hosted run 41269900460 found none of the fragments `not a bot`, `Join this channel`, `terminated`,
  `Proxy Authentication` or `Tunnel connection` anywhere in the archive. An addition to the list needs an observed
  failure on record. The review fails if a phrase is in both lists, or if a site phrase turns up in the archive, which
  moves it to the checked list. **One candidate moved:** `Sign in to confirm your age` is replaced by `confirm your
  age`, the wording yt-dlp itself matches (`extractor/youtube/_video.py:2901`), in the checked list.
- **The order of reasons:** `bot_check` joins `age_restricted` before `login_required`, because yt-dlp adds its
  sign-in hint to any site message that says "sign in" (`extractor/youtube/_video.py:4064-4066`), and the hint is a
  `login_required` phrase. Checked phrases come first within a reason.
- **The hosted proof of these arguments:** run 41269900460 ran the pinned yt-dlp on a made-up video from a local HTTPS
  server on the runner. The comma selection took both streams, the pair wrote `vsift-page.137.mp4` and
  `vsift-page.140.m4a` with exit 0, and a failed second stream exited 1. All 28 flags and 5 filter fields were found.

## Consequences

- VSift makes network requests for the first time outside `setup install`, only on
  `ingest --url`. The threat model gains the acquisition threats (SEC-06 and SEC-31 applied to
  a chosen link, rebinding, proxies, token leaks, a stranger choosing the bytes FFmpeg
  decodes, which reassesses accepted media-parser residuals such as L-144 and adds
  acquired-style files to the hostile-media campaign).
- The skill's "do not use for streaming URLs" changes in one place, with a trial freeze at the
  `0.3.0` cut.
- L-046's "URLs are rejected" changes; L-208 (proxy-resolved addresses) and L-209 (yt-dlp's
  own connections are outside VSift's address policy; extractor breakage between releases)
  are added.
- An organisation-wide switch that turns acquisition off (#409's open decision) is not in
  `0.3.0` and comes in `1.0` (the maintainer, 2026-10-23); acquisition happens only when a person or agent asks for it with `--url`.
