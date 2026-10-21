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
- **Redirects:** at most three, each re-checked (HTTPS, port, no credentials, address).
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
  with the existing margin. The 4-hour duration is checked by the probe afterwards.
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
  `redirect_policy`, `http_status`, `proxy_auth`, `offline`, `stalled`, `deadline`, `size`,
  `partial`, `compressed`, `web_page`, `not_a_video`, `truncated`, `disabled_by_policy`,
  `strict_isolation`. `DOWNLOAD_FAILED` stays the setup code (its fixed message is about the
  reviewed publisher). Every remediation names the online step and its cause and says local
  files still work; none echoes the URL, the server's text or proxy credentials. `web_page`
  names the optional component (section 4) with `required_authority: "user_confirmation"`.

### 4. Public video pages (optional managed yt-dlp, #409)

- **Routing:** `ingest --url` applies section 3's checks first. When the link answers with a
  web page and the component is installed, the link (address-checked) goes to yt-dlp; without
  it the refusal is `web_page` with the command that adds it. VSift keeps no list of sites.
- **The pin (maintainer, 2026-10-21):** yt-dlp stable `2026.08.19`, `yt-dlp_linux`,
  40,446,224 bytes, SHA-256 `58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a`
  (GitHub's asset digest), managed on Ubuntu 24.04 x86-64 only, like every managed component
  (ADR 0023 decision E). The hosted review checks the digest against the release's
  GPG-signed `SHA2-256SUMS` with the key in yt-dlp's repository; the install checks the
  pinned digest, so no OpenPGP code runs on a user's machine. Not a nightly. Windows and macOS
  use a yt-dlp the user supplies (`setup configure yt_dlp --executable <path>`), reported as
  user-managed with its version and digest. **The update path (maintainer):** pins move with
  VSift releases in `0.3.0`; a signed component manifest refreshed between releases is `1.0`'s.
- **Licence:** yt-dlp's source is under the Unlicense, but its PyInstaller executables include
  GPLv3+ code, so the file VSift fetches is GPL-3.0-or-later as a combined work. VSift never
  redistributes it: the publisher's file is fetched onto the user's machine at the user's
  request and run as a separate process, with no linking, so VSift's MIT/Apache licence is
  unaffected. The plan, `setup check` and the third-party notices say so with the notice and
  source URLs. Downloading from a site is subject to its terms, which is the user's
  responsibility; the plan and the documentation say so.
- **JavaScript runtime (maintainer: option 1):** YouTube needs one; the official executables
  already include `yt-dlp-ejs`, so no remote component is fetched. A managed **Deno 2.9.7**
  (`deno-x86_64-unknown-linux-gnu.zip`, 41,596,794 bytes, SHA-256
  `c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490`, MIT) comes with every
  managed yt-dlp; yt-dlp runs it with no file or network permission, so the page's player
  JavaScript runs sandboxed. It needs a bounded reader for a one-entry zip (deflate through
  `flate2`, already in the tree; a new parser with its own fuzz target). yt-dlp is always told
  `--no-js-runtimes --js-runtimes deno:<managed path>`, so a runtime on `PATH` is never used.
  No system runtime is needed.
- **Fixed invocation:** explicit executable and argument array, no shell; an empty environment
  except `TMPDIR`/`TEMP` inside the session's work directory (the one-file build unpacks
  there, inside VSift's own area, removed with the session) and `YTDLP_NO_PLUGINS=1`;
  `--ignore-config --no-plugin-dirs --no-update --no-remote-components --no-cookies
  --no-cookies-from-browser --no-playlist --no-batch-file --max-downloads 1 --abort-on-error
  --no-exec --restrict-filenames --no-mtime --downloader native --use-extractors <qualified
  list>`, a `--match-filters` that refuses live, upcoming and just-ended live streams, a
  format selection of HTTPS files in MP4 or WebM, and the link after `--`. Phase 1 runs
  `--simulate --dump-single-json` (standard output at most 8 MiB, parsed strictly, its own
  fuzz target) and refuses playlists, channels, live and upcoming streams, `private`,
  `needs_auth`, `premium_only` and `subscriber_only` availability, more than 4 hours and any
  protocol but HTTPS. Phase 2 downloads into the session's work directory; VSift's own caps
  stop the process tree; exactly one regular file (or, below, two) with admitted magic is
  accepted.
- **Separate audio and video (maintainer: join them):** when a page offers only separate
  streams, both are downloaded into the session's work directory, each checked for admitted
  magic, and joined into the session's source slot by the pinned FFmpeg as a **stream copy
  only** (`-c copy`, no re-encoding, no filters; fixed arguments, the `file` protocol, the
  `mov` or `matroska` demuxer only), under the same time and size caps. FFmpeg then demuxes
  two hostile inputs, the same exposure as any ingest (the threat review says so); this is
  the first use of FFmpeg as a muxer, which adds the `mp4` and `matroska` muxers to the scan
  scope of every reading.
- **Typed refusals:** a fixed set of yt-dlp's own error phrases is read, never echoed, to
  choose a reason: `login_required`, `bot_check`, `drm`, `age_restricted`, `region_blocked`,
  `private`, `removed`, `unsupported_site`, `extractor_broken`, `js_runtime_missing`,
  `network`, a limit; anything else is `online_step_failed`, which still says local files
  work. A hosted check finds each phrase in the pinned tag's source archive, so a re-pin that
  changes one fails.
- **Setup:** optional components `yt_dlp` and `js_runtime_deno`, never in the default plan;
  `setup plan --profile desktop --include yt_dlp` (the digest covers the choice); the plan
  lists every left-out optional component with its purpose, size, licence, a terms notice and
  the command that adds it, and carries `operator_decision_required` so an agent asks the
  operator before accepting a plan; the transaction installs the core first and the optional
  part after it, and an optional failure never fails the core or `setup check`'s local-video
  profile (#409's recommended result).

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
  a committed test CA and leaf for `video.example.test`, trusted only through a new
  development-only feature, `link-test-hooks`, refused in release builds and by the
  governance check (as `install-test-hooks` is), which also injects a test resolver and the
  loopback exception for the test route only. Every acceptance case of #407 is covered, and
  each leaves no partial file and no session. A production-build test proves loopback and
  private addresses are refused with no hook. Redaction tests put a sentinel token in a
  query, a fragment and a path and find it in no output, record, bundle or log.
- A standard-library fake yt-dlp (a test binary, like `vsift-smoke-fixture`) records its
  arguments and environment and replays recorded JSON, standard error and output files: one
  file, two streams to join, an output outside the work directory, two files, a wrong
  container, an oversize file, a hung process. A planted `yt-dlp.conf` and plugin directory
  are checked against the real binary in the opt-in hosted run only.
- **Live checks** are a manual hosted workflow against NASA public-domain items only: RM09
  on `images-assets.nasa.gov` (direct), RM06 on the Internet Archive (direct, through its
  redirects to storage nodes), RM06's Internet Archive page through the managed yt-dlp, and
  optionally one NASA video on YouTube, where a bot wall on a hosted runner is expected and
  checks the typed refusal. Wikimedia refuses neutral user agents and is never a target.
  Nothing is downloaded to a development machine.

## Delivery

Three pull requests, each standing on its own: **#407's slice** (sections 1 to 3, 5 for direct links and
6's first part: the command, the address policy, the origin, `SOURCE_FETCH_FAILED` and its tests), then
**#409's setup half** (the optional components, the plan's left-out list and `operator_decision_required`,
the core and optional split) and **#409's acquisition half** (the invocation, the refusals, joining, the page
origin). This ADR is written once for all three; each pull request says which sections it delivers.

*Implementation note, 2026-10-21 (#407's pull request):* sections 1 to 3, the direct-link half of section 5
and the first part of section 6 are built: `vsift ingest --url` and `Engine::ingest_link`, the domain values
(`SourceLink`, `DownloadLimits`, `SourceFetchReason`, `SourceOrigin`), the fetch in
`crates/vsift-infrastructure/src/link_acquisition.rs` (the address policy, the resolver, the proxy rule, the
response and body checks) writing into the session's source slot (`SourceSnapshot::stage_link`), the stored
`origin` lifecycle member, `origin` in `ingest` and `session status`, `SOURCE_FETCH_FAILED` with
`error.reason`. The development-only link test route reuses the `install-test-hooks` feature rather than a new
one, so the governance check's existing rule covers it. The skill is not changed in this slice: its "do not use
for streaming URLs" is still true of pages and streams, and a skill change voids the trial freeze, so it is made
once, at the `0.3.0` freeze, with #409's wording.

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
  `0.3.0`; acquisition happens only when a person or agent asks for it with `--url`.
