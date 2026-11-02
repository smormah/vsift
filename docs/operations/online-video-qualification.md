# Online video: the opt-in platform qualification (#409)

Status (2026-11-02, `0.3.0`, draft): **the search found no pair.** This page names the public videos the
qualification may use, with the licence of each, and will state what a hosted run saw. It is the record the
workflow's artifact backs; [L-209](../planning/known-limits.md#l-209) states what stays unseen.

Online video on public pages is an optional component ([ADR 0028](../decisions/0028-online-sources.md)). The
ordinary tests never reach the network and use a fake yt-dlp. This qualification is the one place the page path
meets real sites, from a disposable hosted runner, dispatch-only, in the `Online links live` workflow. It uses no
token, credential, cookie or sign-in. A bot wall or a login wall is a result to record, never something to get round.

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
