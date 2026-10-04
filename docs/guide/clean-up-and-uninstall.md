# Clean up and uninstall

VSift keeps a private copy of each video you open, plus everything it extracts. This page shows how to
see what it has stored, remove it, and remove VSift itself. Nothing here touches your original videos or
the bundles you saved.

**The result:** no sessions left on the machine, or VSift gone altogether, with your own files exactly where
they were.

## 1. Sessions end by themselves, but only on paper

A session is disposable. It expires 24 hours after it was opened (the `expires` time is on the last line of
every result); `vsift session renew <session>` moves the expiry to 24 hours from now, never past seven days
after it was opened. After the expiry VSift treats the session as expired and refuses to use it. The
**files stay on the disk** until a cleanup removes them: VSift has no background service, so nothing
runs on its own. The same holds for a session you close.

Where the files are (the per-user folder `VSift-sessions`; the [install guide](../operations/install.md#8-uninstall)
lists every folder VSift writes):

| System | Sessions |
| --- | --- |
| Windows | `%LOCALAPPDATA%\VSift-sessions` |
| macOS | `~/Library/Caches/VSift-sessions` |
| Linux | `$XDG_CACHE_HOME/vsift-sessions`, or `~/.cache/vsift-sessions` |

## 2. See what is there

<!-- check -->
```console
$ vsift ingest ./F04-speech.mp4
Opened session ses_6e9a8a3d…
Source: src_sha256_c93562cb… (140234 bytes)
Generation: 1 (process_crash_consistent)
Transcript: none imported

Lifecycle: ephemeral, expires 2026-10-05T18:40:00Z
$ vsift session list
Sessions:
  ses_6e9a8a3d…  open  expires 2026-10-05T18:40:00Z
More on the next page: repeat the command with --cursor 127
```

`session list` walks VSift's session folder in bounded pages. A page can hold few or no sessions and still
end with `More on the next page`: keep repeating the command with the number it gives until that line no
longer appears. `vsift session status <session>` shows one session, with its size and any long-running work.

## 3. Close a session when you are done

<!-- check -->
```console
$ vsift session close <session>
Closed session ses_6e9a8a3d…
State: closed
Source: src_sha256_c93562cb… (140234 bytes)
Artifacts: 1 (1000 bytes)
Generation: 2

Lifecycle: ephemeral, expires 2026-10-05T18:40:00Z
```

Closing is polite but optional. A closed session cannot be used again and its files are fair game for the
next cleanup. If work is still running in the session (a transcription, for instance), close answers `BUSY`: wait and
try again.

## 4. Remove the sessions that are finished

Look first. `--dry-run` removes nothing and says which sessions the cleanup would take:

<!-- check -->
```console
$ vsift session clean --expired --dry-run
Session cleanup, dry run: nothing was removed.
  ses_6e9a8a3d…  eligible
More on the next page: repeat the command with --cursor 127
```

`eligible` means closed or expired. A session that is still open and unexpired is `ineligible` and is left
alone. Then do it:

<!-- check -->
```console
$ vsift session clean --expired
Session cleanup:
  ses_6e9a8a3d…  removed
More on the next page: repeat the command with --cursor 127
$ vsift session list
No sessions.
```

Cleanup removes every closed, expired or abandoned session of your account, not only the ones from today.
A session someone else is using at that moment is reported busy and skipped: run it again later. An
interrupted `ingest` can leave a session listed as `initializing`; cleanup removes it once it has been idle long
enough. VSift does not claim to erase data securely: the files are deleted the ordinary way.

## 5. What VSift never removes

- **Your original videos and transcript files.** VSift works on private copies.
- **Retained bundles.** A folder made with `session retain` is yours, outside automatic cleanup; delete it
  yourself ([keep and share evidence](keep-and-share-evidence.md)).
- **Tools you registered** with `setup configure` (FFmpeg, FFprobe, whisper.cpp, a model). VSift only
  removes the managed copies it installed itself, and only when you ask.

## 6. Uninstall VSift

The install guide has the full procedure, with the folder list for each system
([section 8](../operations/install.md#8-uninstall)). In short, in this order:

1. If you used the managed tools (Ubuntu 24.04 only), remove them while `vsift` still exists:
   `vsift setup remove ffmpeg_ffprobe`, `vsift setup remove whisper_cli`, `vsift setup remove whisper_model`.
2. Remove the package: `npm uninstall --global vsift-cli` (or the same with pnpm, Bun or Yarn), or delete the
   folder you extracted the archive into.
3. Delete VSift's own folders if you want them gone: the configuration folder, the sessions folder and, on
   Linux, the managed-tools folder.

Each step is separate, and none of them touches your videos.
