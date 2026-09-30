# Session lifecycle: close, retain, clean

VSift investigations are disposable by default. `ingest` copies the video into a
private session under the user's per-user cache; the session and everything
extracted from it expire after 24 idle hours (renewals never extend a session beyond
seven days from its opening). Expiry makes a session unusable at once, but its files
are removed only when `session clean` runs; VSift has no background service and
promises no secure erasure. Nothing is ever deleted from the user's own video.

## The user's lifecycle policy

Before or during the investigation, the user may say what should happen afterwards.
You may record it as `lifecycle.policy` `user_stated` (otherwise `default`); the
member is optional.

| The user said | You do at CLOSE_OR_RETAIN |
| --- | --- |
| Nothing (`default`) | Leave the session open so the user can check your citations; report `lifecycle.expires_at` and how to close it. |
| "Close it" / "clean up when done" | `vsift session close <session> --json` on your own session. |
| "Keep the evidence" / "save a bundle to D" | After your last evidence command, `vsift session retain <session> --output <new-directory> --json` with the new directory the user named; `--include-source` only if they asked for the video to be included. Then `vsift bundle validate <bundle-directory> --json`. The bundle holds the session as it was when you retained it: cite nothing you extract later. |
| "Keep it open longer" | `vsift session renew <session> --json` while the session is still open; an expired session cannot be renewed. |
| "Reopen if it expires" | As stated in [resume.md](resume.md). |

Closing makes the session's image and audio paths invalid for reading; its files
remain until cleanup. A retained bundle is a new private directory outside automatic
cleanup; it never contains a local path. Retaining never moves or deletes the video.

## Cleanup routine

Cleanup removes every session of the per-user root that VSift can prove expired,
closed or abandoned, not only yours, so it runs only when the user asks. Show the
user what it would remove first:

```console
vsift session clean --expired --dry-run --json
```

The result is one bounded page; repeat with the `next_cursor` it returns
(`--cursor <n>`) until it is null. Then, with the user's go-ahead:

```console
vsift session clean --expired --json
```

A session another process is using is reported busy and left alone; run the cleanup
again later. An interrupted retain can leave an incomplete private directory, which
`bundle validate` rejects; tell the user where they asked it to be written so they
can inspect or remove it themselves. Cleanup does not erase backups, snapshots or
copies made elsewhere.

## In the handoff

`lifecycle.action` is `left_open`, `closed`, `retained`, `expired` (it expired before
you finished) or `not_opened` (no session was created); it is required. The optional
`lifecycle.mode` and `lifecycle.expires_at`, when you give them, are copied from the
last VSift result for the session.
