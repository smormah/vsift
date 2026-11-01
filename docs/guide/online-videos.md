# Online videos

VSift works on a video on your machine. From `0.3.0` it can also fetch one for you from a link, once, into the
session, and from then on everything works offline, as for a file. This page says what works with nothing extra,
what the optional part adds, how to add it and take it away, and what to keep in mind.

**The result:** an investigation of a video you only had a link to, or a clear answer why VSift could not fetch it,
with your local files untouched either way.

## What works with nothing extra

A direct `https://` link to the video file, one that ends in `.mp4`, `.mov`, `.mkv` or `.webm` or that downloads a
video when you open it, needs nothing more than VSift:

```console
vsift ingest --url <link>
```

VSift downloads it once into the session, with a limit of 4 GiB and 30 minutes (`--max-download-bytes` and
`--max-download-seconds` raise them). `--url -` reads the link from standard input, so a link with a private token
stays out of your shell history. The link is never stored: the session keeps only the host and a fingerprint of it
(`Fetched from:`).

## What the optional part adds

A link to a video's **web page**, such as a video site's page, is not the video file. VSift refuses it
(`web_page`) unless a tool that can read such pages is set up: yt-dlp, with a JavaScript runtime that some sites
need to work (Deno). With it, the same command works:

```console
vsift ingest --url <the page's link>
```

VSift reads what the page offers, fetches one finished video of at most four hours, and joins separate sound and
picture when the site serves them apart. Links that need a sign-in, links to a private or protected video, and links
to your own network never work, and a playlist or channel, a live stream and a video over four hours are refused. When it fails, the answer says why
([troubleshooting](troubleshooting.md#source_fetch_failed-a-video-link-could-not-be-fetched)); your local files are
not affected.

## Add it

Nothing here is installed unless you ask. If an AI assistant is setting VSift up for you, it should tell you about
this option and ask you first.

- **On Ubuntu 24.04 (x86-64), VSift installs and checks it for you.** It is about 83 MB in two downloads: yt-dlp
  (GPL-3.0-or-later) and Deno (MIT). From the setup command:

  ```console
  vsift setup --include web-videos
  ```

  or, step by step, with a plan you read before you accept it:

  ```console
  vsift setup plan --profile desktop --include online_links --json > plan.json
  vsift setup install --plan plan.json --accept-plan <the plan digest>
  ```

  The core tools are installed first. If this part cannot be installed (a proxy that blocks its download, say), the
  core stays installed and ready, the result says only that online videos are not available and what to try, and
  you can run the same command again.
- **On Windows and macOS, you supply it.** VSift does not install or review a yt-dlp there. Install one yourself,
  and register it with its absolute path, and the JavaScript runtime it needs if you have one:

  ```console
  vsift setup configure yt-dlp --executable <absolute-path-to-yt-dlp>
  vsift setup configure js-runtime --executable <absolute-path-to-deno>
  vsift setup check
  ```

  Keeping a file you supplied current is yours to do. Sites change, and an old yt-dlp stops working with them.
  The same two commands work on Ubuntu when you would rather use your own.

`vsift setup check` has a line, "Web videos", that says whether it is available. It does not change whether your
setup for local videos is complete.

## Take it away

- **A part VSift installed:** `vsift setup remove yt_dlp` and `vsift setup remove js_runtime_deno`. Local
  investigation is as ready as before.
- **A part you supplied:** VSift only records its path and has no command that forgets it. Remove the file, or the
  registered paths with VSift's configuration folder (the [install guide](../operations/install.md#8-uninstall)
  names it); that folder also holds your other registered tools.

## What to keep in mind

- **Each site's terms apply.** Downloading a video may be against a site's terms, whoever asks for it. VSift is
  not a way round them, and it does not try to get past sign-ins or bot checks; a site that refuses is reported as
  refusing.
- **Sites change.** A page that worked can stop working until yt-dlp is updated. VSift updates its installed one
  only with a new release of VSift, and never by itself.
- **An organisation can leave it out.** VSift installs this part only when someone includes it, so a rollout that
  does not include `online_links` never has it, and the plan for a worker host does not offer it. A setting that
  forbids it even when asked is not in this release.
- **The link is not kept.** It is used to fetch the video from the site you named, through your network and proxy
  settings. The session keeps no copy of it, and no title, uploader or address of the video's format.
