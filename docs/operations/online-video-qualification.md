# Online video: the opt-in platform qualification (#409)

Status (2026-11-02, `0.3.0`): **done as far as a hosted runner can take it. No real site has given `joined: audio_video`,
from a hosted runner or from anywhere else.** This page names the public videos the qualification uses, with the licence
of each, states what the hosted run saw, and says what has not been seen. The record of each run is that run's artifact;
[L-209](../planning/known-limits.md#l-209) states what stays unseen. #409 stays open.

Online video on public pages is an optional component ([ADR 0028](../decisions/0028-online-sources.md)). The
ordinary tests never reach the network and use a fake yt-dlp. This qualification is the one place the page path
meets real sites, from a disposable hosted runner, dispatch-only, in the `Online links live` workflow. It uses no
token, credential, cookie or sign-in. A bot wall or a login wall is a result to record, never something to get round.

## How it runs

- **`qualify`:** dispatch `Online links live` with `qualify` set, once for a change to page fetching and at each re-pin of
  yt-dlp, beside the existing `youtube` input. It installs the managed pair, then takes each page below through
  `ingest --url`. The record, `online-video-qualification-record`, is kept for 90 days: the date, the runner, the yt-dlp
  and runtime versions as `setup check` reports them, each page's outcome (fetched and how it was joined, or the typed
  reason), and `pair_seen`. It holds no link. The test fails only when VSift itself misbehaves (a link printed, an
  untyped failure, the refusal case giving another reason), never because a site walled the runner.
- **`candidates`:** the information phase only, nothing downloaded. It reads up to eight page links with the pinned
  yt-dlp and phase 1's selection and says whether each offers a pair, one file or a refusal
  (`tools/online-video-qualification/search_formats.py`).
- **Only `joined: audio_video` counts as a real pair.** The `youtube` input's check also passes on the typed
  `bot_check`, so a pass there proves nothing about a pair.

## The named list

Every video here is short, public, and either the project's own choice of a public-domain work or carries a licence
that allows this use. The record holds no link with a token.

| Id | Video | Where | Licence |
| --- | --- | --- | --- |
| Q1 | NASA, "Watch NASA's Perseverance Rover Land on Mars!" | `https://www.facebook.com/NASA/videos/438313177583525/` | Public domain (US government work), per [NASA's media guidelines](https://www.nasa.gov/nasa-brand-center/images-and-media/); the site's terms apply |
| Q2 | NASA, "What is #Artemis? The twin sister of Apollo" | `https://www.facebook.com/NASA/videos/645832639583660/` | The same |
| Q3 | Blender, "Blender 4.0 - Reel" (85 s) | `https://video.blender.org/videos/watch/ac71efff-5e26-440b-8c38-dd1f5b598e02` | Attribution (CC BY), as the instance states |
| Q4 | "A Year (and a half) of Continuwuity" (527 s) | `https://media.ccc.de/v/matrix-conference-2026-99063-a-year-and-a-half-of-continuwuity` | CC BY-SA 4.0, as the site states |
| Q5 | "Editing basics - watchlists" (136 s), the real-media set's RM01 | `https://commons.wikimedia.org/wiki/File:Editing_basics_-_watchlists.webm` | CC BY-SA 3.0, [the file's page](https://commons.wikimedia.org/wiki/File:Editing_basics_-_watchlists.webm) |
| Q6 | NASA video page `nhq20230118armd01` | `https://www.nasa.gov/video-detail/nhq20230118armd01` | Public domain (US government work), per NASA's media guidelines |
| R1 | Blender Showreels, a playlist page (the refusal case) | `https://video.blender.org/w/p/cQ4v4uoPbLdnhFrqsEx69m` | Attribution (CC BY), as the instance states |

The list is also the test's: `LISTED` in `crates/vsift-cli/tests/link_live.rs`. A change to one is a change to both.
## The search for a real pair of streams (2026-11-02)

One hosted dispatch (run 41442126342, Ubuntu 24.04, yt-dlp 2026.08.19, the information phase only, nothing downloaded)
read Q1 to Q6 with phase 1's selection. **No page offered a separate audio stream next to a video stream** over https:

| Id | Result |
| --- | --- |
| Q1 | The extractor could not parse the page (`Cannot parse data`); no formats |
| Q2 | Combined `https` mp4 (2) and one video-only mp4, no audio-only stream: one file |
| Q3 | Combined `https` mp4 (4) and HLS: one file |
| Q4 | Combined `https` mp4 and webm, audio-only mp3 and opus only (not the container the selection takes): one file |
| Q5 | Combined `https` webm (3), one video-only mov: one file |
| Q6 | A page the generic reader found no video in |

YouTube, the one site that does serve separate streams, met a bot wall from a hosted runner on 2026-11-01 (dispatch
41281919811); it is not searched again here.

## What the qualification run saw

Dispatch 41444797025 (2026-11-02, head `ebe5dde5`, Ubuntu 24.04), through the managed pair `yt-dlp-2026.08.19-linux-x86_64`
and `deno-2.9.7-x86_64-linux-gnu`. The record is that run's artifact `online-video-qualification-record` (90 days);
`pair_seen` is `false` and `problems` is empty. An earlier dispatch (41443956829) failed on a wrong path in the test
itself, before it read any page, and is not a result.

| Id | Outcome | What it shows |
| --- | --- | --- |
| Q1 | Refused as `extractor_broken` | The extractor could not parse the page (the same text the search saw); the typed refusal is the right one |
| Q2 | Fetched, `single_file`, 139,724 bytes | A page on a social site through yt-dlp: the one file the site offers, no audio stream apart |
| Q3 | Fetched, `single_file`, 19,555,969 bytes | A PeerTube instance: a combined mp4 |
| Q4 | Fetched, `single_file`, 142,625,911 bytes | media.ccc.de: a combined mp4 |
| Q5 | Refused as `http_status` | The site answers VSift's own first request with an HTTP error, so the link never reaches yt-dlp (the real-media set records that this site refuses the tool's own requests); the typed refusal is the right one |
| Q6 | Refused as `playlist` | The generic reader found a list of entries on the page, not one video |
| R1 | Refused as `playlist` | The playlist case the qualification requires, from a real page |

Seen, then: a page fetched through the managed pair on three sites and written as one file; four typed refusals
from real pages (`extractor_broken`, `http_status`, `playlist` twice); no link in any output. Not seen: a pair.

## Not run, and why

| Case | Why it was not run | The fake's test that covers it |
| --- | --- | --- |
| A private video | No public private page exists; reaching one needs a sign-in, which is never used | `what_may_not_be_fetched_is_refused_with_its_reason` (`ytdlp_info.rs`), the `private` case; `every_phrase_gives_its_typed_reason_and_is_not_echoed` (`page_acquisition.rs`) |
| A sign-in page | The same: a page that asks for a login is reached only by one with an account | The `needs_auth`, `premium_only` and `subscriber_only` cases of the same test, and the login phrases |
| A DRM-protected page | No public page is known to a hosted runner that reports DRM without a sign-in | The `Drm` phrases of `YTDLP_PHRASES` in the phrase test |
| A real pair of streams | See below | `separate_streams_are_joined_and_an_unlisted_id_is_not_kept` and `webm_streams_are_joined_as_matroska` (`page_acquisition.rs`), the verified FFmpeg joining the fake's two streams |

## No real pair has been seen

Joining separate audio and video has run only on the fake yt-dlp and on a local probe with a made-up video. No real
site has given `joined: audio_video`. The only site known to serve separate streams over https, YouTube, refuses
hosted runners with a bot check (dispatch 41281919811 on 2026-11-01), and one search of six other pages on
2026-11-02 found none that offers an audio stream apart from a video stream. A run from an ordinary network is the
next step; it needs the maintainer's go naming the machine, the file, the link and the size.
