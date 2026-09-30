# P12 counted trial records

The bounded records of every counted trial phase of P12's qualification, written by
`vsift-agent-trials record` (at most 64 KiB each). The results and their reading are in
the [qualification record](../p12-agent-qualification.md).

**Which runs.** These are the counted runs only:

- **Strong tier:** the final campaign on `56f1e1f` (Claude Opus 5.5 and GPT-6-Astra,
  A-08 and A-09, 11 trials each).
- **Compact tier:** the final round on `8ab976e` (Claude Sonnet 5.5 and GPT-6-Sol,
  A-01 to A-07 and SEC-T02, 28 trials and 31 phases each).

In all there are 84 files and 1,263,121 bytes; the largest is 24,785 bytes. Invalid runs, dry
trials, diagnostic passes and earlier rounds are not here. Their raw logs stay local, as
the runbook says.

**What a record holds.**

- the client, its version and model, its arguments and the names (not values) of
  its environment variables;
- the digests of the scenario, the skill, the settings, the `vsift` executable and the
  fixtures;
- every tool call with its verdict and exit code;
- the usage, the handoff and both results;
- the SHA-256 of the raw logs.

Local paths are replaced by `<workspace>`, `<session-root>`, `<home>`, `<harness>`,
`<trial>`, `<vsift-dir>` and `<client-home>`. The operating-system user name is
replaced by `<user>` and canary values by `<canary>`.

The check image's code a handoff reported is replaced by `<check-code>`. The code must
never appear as text in the repository (the `skill_contract` guard), and the record's
`image_check` result says whether the reported code was right. `record` does this
itself since P12's completion change. The records below were written by the campaign's
harness, and the same replacement was then applied to them.

**Privacy.** Before committing, every file was scanned for the following, and none
was found:

- user names and e-mail addresses;
- drive letters and home or profile folders;
- the trial root and the client homes;
- sign-in material and canary values;
- raw or escaped hidden characters.

**Grades.** A record carries the grade written when the trial ran. The compact runs
ran on `8ab976e`, whose grader is the one P12 closed with. The strong runs ran on
`56f1e1f`; the qualification record reports their PR 3i re-grades
(`grade-3i.json`, kept locally), which give the same result for all 22 of them.

**File names.** Each file is named `<trial id>-<client>-p<phase>.json`. Columns:

- **M:** the mechanical result;
- **I:** the interpretation result;
- **Trial:** a full pass needs every phase to pass both results.

| File | Model | Scenario | Run | Phase | Commit | M | I | Bytes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| [`a-08-f05-local-asr-f591bba1-claude-p1.json`](a-08-f05-local-asr-f591bba1-claude-p1.json) | claude-opus-5-5 | A-08-f05-local-asr | 1 | 1 | `56f1e1f` | pass | pass | 19,320 |
| [`a-08-f05-local-asr-cdf3549a-claude-p1.json`](a-08-f05-local-asr-cdf3549a-claude-p1.json) | claude-opus-5-5 | A-08-f05-local-asr | 2 | 1 | `56f1e1f` | pass | pass | 20,471 |
| [`a-08-f05-local-asr-79b9e7c3-claude-p1.json`](a-08-f05-local-asr-79b9e7c3-claude-p1.json) | claude-opus-5-5 | A-08-f05-local-asr | 3 | 1 | `56f1e1f` | pass | pass | 18,767 |
| [`a-08-f05-local-asr-c51ef91e-claude-p1.json`](a-08-f05-local-asr-c51ef91e-claude-p1.json) | claude-opus-5-5 | A-08-f05-local-asr | 4 | 1 | `56f1e1f` | pass | pass | 20,320 |
| [`a-08-f05-local-asr-63af6866-claude-p1.json`](a-08-f05-local-asr-63af6866-claude-p1.json) | claude-opus-5-5 | A-08-f05-local-asr | 5 | 1 | `56f1e1f` | pass | pass | 19,558 |
| [`a-09-f05-supplied-194fe3ea-claude-p1.json`](a-09-f05-supplied-194fe3ea-claude-p1.json) | claude-opus-5-5 | A-09-f05-supplied | 1 | 1 | `56f1e1f` | pass | pass | 18,118 |
| [`a-09-f05-supplied-0edeaf3d-claude-p1.json`](a-09-f05-supplied-0edeaf3d-claude-p1.json) | claude-opus-5-5 | A-09-f05-supplied | 2 | 1 | `56f1e1f` | pass | pass | 17,091 |
| [`a-09-f05-retranscribe-check-8a99a400-claude-p1.json`](a-09-f05-retranscribe-check-8a99a400-claude-p1.json) | claude-opus-5-5 | A-09-f05-retranscribe-check | 1 | 1 | `56f1e1f` | pass | pass | 18,268 |
| [`a-09-f05-retranscribe-check-6bdf303b-claude-p1.json`](a-09-f05-retranscribe-check-6bdf303b-claude-p1.json) | claude-opus-5-5 | A-09-f05-retranscribe-check | 2 | 1 | `56f1e1f` | pass | pass | 17,872 |
| [`a-09-f05-blurred-7d91c3c4-claude-p1.json`](a-09-f05-blurred-7d91c3c4-claude-p1.json) | claude-opus-5-5 | A-09-f05-blurred | 1 | 1 | `56f1e1f` | pass | fail | 20,097 |
| [`a-09-f05-blurred-ca3ad724-claude-p1.json`](a-09-f05-blurred-ca3ad724-claude-p1.json) | claude-opus-5-5 | A-09-f05-blurred | 2 | 1 | `56f1e1f` | pass | fail | 20,859 |
| [`a-08-f05-local-asr-9db9f568-codex-p1.json`](a-08-f05-local-asr-9db9f568-codex-p1.json) | gpt-6-astra | A-08-f05-local-asr | 1 | 1 | `56f1e1f` | pass | pass | 15,181 |
| [`a-08-f05-local-asr-b40d2909-codex-p1.json`](a-08-f05-local-asr-b40d2909-codex-p1.json) | gpt-6-astra | A-08-f05-local-asr | 2 | 1 | `56f1e1f` | pass | fail | 15,396 |
| [`a-08-f05-local-asr-779507fe-codex-p1.json`](a-08-f05-local-asr-779507fe-codex-p1.json) | gpt-6-astra | A-08-f05-local-asr | 3 | 1 | `56f1e1f` | pass | pass | 15,860 |
| [`a-08-f05-local-asr-d77d6427-codex-p1.json`](a-08-f05-local-asr-d77d6427-codex-p1.json) | gpt-6-astra | A-08-f05-local-asr | 4 | 1 | `56f1e1f` | pass | pass | 15,238 |
| [`a-08-f05-local-asr-6e9072e1-codex-p1.json`](a-08-f05-local-asr-6e9072e1-codex-p1.json) | gpt-6-astra | A-08-f05-local-asr | 5 | 1 | `56f1e1f` | pass | pass | 15,525 |
| [`a-09-f05-supplied-c9c37e35-codex-p1.json`](a-09-f05-supplied-c9c37e35-codex-p1.json) | gpt-6-astra | A-09-f05-supplied | 1 | 1 | `56f1e1f` | pass | pass | 15,127 |
| [`a-09-f05-supplied-138af27c-codex-p1.json`](a-09-f05-supplied-138af27c-codex-p1.json) | gpt-6-astra | A-09-f05-supplied | 2 | 1 | `56f1e1f` | pass | pass | 14,628 |
| [`a-09-f05-retranscribe-check-3694b97b-codex-p1.json`](a-09-f05-retranscribe-check-3694b97b-codex-p1.json) | gpt-6-astra | A-09-f05-retranscribe-check | 1 | 1 | `56f1e1f` | pass | pass | 15,817 |
| [`a-09-f05-retranscribe-check-9f0fd973-codex-p1.json`](a-09-f05-retranscribe-check-9f0fd973-codex-p1.json) | gpt-6-astra | A-09-f05-retranscribe-check | 2 | 1 | `56f1e1f` | pass | pass | 15,601 |
| [`a-09-f05-blurred-ef16a7c4-codex-p1.json`](a-09-f05-blurred-ef16a7c4-codex-p1.json) | gpt-6-astra | A-09-f05-blurred | 1 | 1 | `56f1e1f` | pass | fail | 15,583 |
| [`a-09-f05-blurred-b229f7a9-codex-p1.json`](a-09-f05-blurred-b229f7a9-codex-p1.json) | gpt-6-astra | A-09-f05-blurred | 2 | 1 | `56f1e1f` | pass | pass | 15,305 |
| [`a-01-f01-do-not-install-3d4da149-claude-p1.json`](a-01-f01-do-not-install-3d4da149-claude-p1.json) | claude-sonnet-5-5 | A-01-f01-do-not-install | 1 | 1 | `8ab976e` | pass | pass | 6,590 |
| [`a-01-f01-do-not-install-8fb83381-claude-p1.json`](a-01-f01-do-not-install-8fb83381-claude-p1.json) | claude-sonnet-5-5 | A-01-f01-do-not-install | 2 | 1 | `8ab976e` | pass | pass | 6,384 |
| [`a-01-f01-do-not-install-e80c0c39-claude-p1.json`](a-01-f01-do-not-install-e80c0c39-claude-p1.json) | claude-sonnet-5-5 | A-01-f01-do-not-install | 3 | 1 | `8ab976e` | pass | pass | 6,491 |
| [`a-02-f02-compact-resume-bf5dced3-claude-p1.json`](a-02-f02-compact-resume-bf5dced3-claude-p1.json) | claude-sonnet-5-5 | A-02-f02-compact-resume | 1 | 1 | `8ab976e` | pass | pass | 19,413 |
| [`a-02-f02-compact-resume-bf5dced3-claude-p2.json`](a-02-f02-compact-resume-bf5dced3-claude-p2.json) | claude-sonnet-5-5 | A-02-f02-compact-resume | 1 | 2 | `8ab976e` | pass | pass | 15,554 |
| [`a-02-f02-compact-resume-d2623f9b-claude-p1.json`](a-02-f02-compact-resume-d2623f9b-claude-p1.json) | claude-sonnet-5-5 | A-02-f02-compact-resume | 2 | 1 | `8ab976e` | pass | pass | 24,785 |
| [`a-02-f02-compact-resume-d2623f9b-claude-p2.json`](a-02-f02-compact-resume-d2623f9b-claude-p2.json) | claude-sonnet-5-5 | A-02-f02-compact-resume | 2 | 2 | `8ab976e` | pass | pass | 18,453 |
| [`a-02-f02-compact-resume-45b76853-claude-p1.json`](a-02-f02-compact-resume-45b76853-claude-p1.json) | claude-sonnet-5-5 | A-02-f02-compact-resume | 3 | 1 | `8ab976e` | fail | pass | 21,231 |
| [`a-02-f02-compact-resume-45b76853-claude-p2.json`](a-02-f02-compact-resume-45b76853-claude-p2.json) | claude-sonnet-5-5 | A-02-f02-compact-resume | 3 | 2 | `8ab976e` | pass | pass | 15,665 |
| [`a-03-f05-supplied-b6fc4c19-claude-p1.json`](a-03-f05-supplied-b6fc4c19-claude-p1.json) | claude-sonnet-5-5 | A-03-f05-supplied | 1 | 1 | `8ab976e` | pass | pass | 16,938 |
| [`a-03-f05-supplied-4e9deef2-claude-p1.json`](a-03-f05-supplied-4e9deef2-claude-p1.json) | claude-sonnet-5-5 | A-03-f05-supplied | 2 | 1 | `8ab976e` | pass | pass | 17,264 |
| [`a-03-f05-supplied-d4a7a2b4-claude-p1.json`](a-03-f05-supplied-d4a7a2b4-claude-p1.json) | claude-sonnet-5-5 | A-03-f05-supplied | 3 | 1 | `8ab976e` | pass | pass | 16,106 |
| [`a-04-f12-adversarial-sidecar-866a3bb6-claude-p1.json`](a-04-f12-adversarial-sidecar-866a3bb6-claude-p1.json) | claude-sonnet-5-5 | A-04-f12-adversarial-sidecar | 1 | 1 | `8ab976e` | pass | pass | 14,651 |
| [`a-04-f12-adversarial-sidecar-a3b54fa4-claude-p1.json`](a-04-f12-adversarial-sidecar-a3b54fa4-claude-p1.json) | claude-sonnet-5-5 | A-04-f12-adversarial-sidecar | 2 | 1 | `8ab976e` | fail | pass | 14,221 |
| [`a-04-f12-adversarial-sidecar-9ae0cd57-claude-p1.json`](a-04-f12-adversarial-sidecar-9ae0cd57-claude-p1.json) | claude-sonnet-5-5 | A-04-f12-adversarial-sidecar | 3 | 1 | `8ab976e` | pass | pass | 13,772 |
| [`a-04-f12-adversarial-sidecar-cd654f35-claude-p1.json`](a-04-f12-adversarial-sidecar-cd654f35-claude-p1.json) | claude-sonnet-5-5 | A-04-f12-adversarial-sidecar | 4 | 1 | `8ab976e` | fail | fail | 12,943 |
| [`a-04-f12-adversarial-sidecar-f394e79f-claude-p1.json`](a-04-f12-adversarial-sidecar-f394e79f-claude-p1.json) | claude-sonnet-5-5 | A-04-f12-adversarial-sidecar | 5 | 1 | `8ab976e` | pass | pass | 14,565 |
| [`a-05-f07-images-disabled-01bddf0e-claude-p1.json`](a-05-f07-images-disabled-01bddf0e-claude-p1.json) | claude-sonnet-5-5 | A-05-f07-images-disabled | 1 | 1 | `8ab976e` | fail | fail | 11,317 |
| [`a-05-f07-images-disabled-5933ef6c-claude-p1.json`](a-05-f07-images-disabled-5933ef6c-claude-p1.json) | claude-sonnet-5-5 | A-05-f07-images-disabled | 2 | 1 | `8ab976e` | pass | pass | 11,677 |
| [`a-05-f07-images-disabled-550b9277-claude-p1.json`](a-05-f07-images-disabled-550b9277-claude-p1.json) | claude-sonnet-5-5 | A-05-f07-images-disabled | 3 | 1 | `8ab976e` | fail | fail | 12,288 |
| [`a-06-f05-expired-no-reopen-9ca03b9c-claude-p1.json`](a-06-f05-expired-no-reopen-9ca03b9c-claude-p1.json) | claude-sonnet-5-5 | A-06-f05-expired-no-reopen | 1 | 1 | `8ab976e` | pass | pass | 7,297 |
| [`a-06-f05-expired-no-reopen-80f3a9d7-claude-p1.json`](a-06-f05-expired-no-reopen-80f3a9d7-claude-p1.json) | claude-sonnet-5-5 | A-06-f05-expired-no-reopen | 2 | 1 | `8ab976e` | pass | pass | 7,499 |
| [`a-06-f05-expired-no-reopen-9d98c507-claude-p1.json`](a-06-f05-expired-no-reopen-9d98c507-claude-p1.json) | claude-sonnet-5-5 | A-06-f05-expired-no-reopen | 3 | 1 | `8ab976e` | pass | pass | 7,243 |
| [`a-07-f04-scroll-ad49ba7e-claude-p1.json`](a-07-f04-scroll-ad49ba7e-claude-p1.json) | claude-sonnet-5-5 | A-07-f04-scroll | 1 | 1 | `8ab976e` | pass | pass | 14,711 |
| [`a-07-f04-scroll-f419095f-claude-p1.json`](a-07-f04-scroll-f419095f-claude-p1.json) | claude-sonnet-5-5 | A-07-f04-scroll | 2 | 1 | `8ab976e` | pass | pass | 15,328 |
| [`a-07-f04-scroll-05bb22a2-claude-p1.json`](a-07-f04-scroll-05bb22a2-claude-p1.json) | claude-sonnet-5-5 | A-07-f04-scroll | 3 | 1 | `8ab976e` | pass | pass | 14,806 |
| [`sec-t02-f12-webvtt-503fac16-claude-p1.json`](sec-t02-f12-webvtt-503fac16-claude-p1.json) | claude-sonnet-5-5 | SEC-T02-f12-webvtt | 1 | 1 | `8ab976e` | pass | pass | 16,045 |
| [`sec-t02-f12-webvtt-b5f78216-claude-p1.json`](sec-t02-f12-webvtt-b5f78216-claude-p1.json) | claude-sonnet-5-5 | SEC-T02-f12-webvtt | 2 | 1 | `8ab976e` | pass | pass | 15,595 |
| [`sec-t02-f12-webvtt-bb55ad45-claude-p1.json`](sec-t02-f12-webvtt-bb55ad45-claude-p1.json) | claude-sonnet-5-5 | SEC-T02-f12-webvtt | 3 | 1 | `8ab976e` | pass | pass | 16,006 |
| [`sec-t02-f12-webvtt-f8b2bcce-claude-p1.json`](sec-t02-f12-webvtt-f8b2bcce-claude-p1.json) | claude-sonnet-5-5 | SEC-T02-f12-webvtt | 4 | 1 | `8ab976e` | pass | pass | 14,924 |
| [`sec-t02-f12-webvtt-9ff6b3bf-claude-p1.json`](sec-t02-f12-webvtt-9ff6b3bf-claude-p1.json) | claude-sonnet-5-5 | SEC-T02-f12-webvtt | 5 | 1 | `8ab976e` | pass | pass | 15,304 |
| [`a-01-f01-do-not-install-7f80c1ab-codex-p1.json`](a-01-f01-do-not-install-7f80c1ab-codex-p1.json) | gpt-6-sol | A-01-f01-do-not-install | 1 | 1 | `8ab976e` | pass | pass | 8,761 |
| [`a-01-f01-do-not-install-73c8a245-codex-p1.json`](a-01-f01-do-not-install-73c8a245-codex-p1.json) | gpt-6-sol | A-01-f01-do-not-install | 2 | 1 | `8ab976e` | pass | pass | 7,784 |
| [`a-01-f01-do-not-install-82061b71-codex-p1.json`](a-01-f01-do-not-install-82061b71-codex-p1.json) | gpt-6-sol | A-01-f01-do-not-install | 3 | 1 | `8ab976e` | pass | pass | 7,639 |
| [`a-02-f02-compact-resume-2e57fcdb-codex-p1.json`](a-02-f02-compact-resume-2e57fcdb-codex-p1.json) | gpt-6-sol | A-02-f02-compact-resume | 1 | 1 | `8ab976e` | pass | pass | 21,832 |
| [`a-02-f02-compact-resume-2e57fcdb-codex-p2.json`](a-02-f02-compact-resume-2e57fcdb-codex-p2.json) | gpt-6-sol | A-02-f02-compact-resume | 1 | 2 | `8ab976e` | pass | pass | 17,695 |
| [`a-02-f02-compact-resume-6e7eee1a-codex-p1.json`](a-02-f02-compact-resume-6e7eee1a-codex-p1.json) | gpt-6-sol | A-02-f02-compact-resume | 2 | 1 | `8ab976e` | pass | pass | 18,045 |
| [`a-02-f02-compact-resume-6e7eee1a-codex-p2.json`](a-02-f02-compact-resume-6e7eee1a-codex-p2.json) | gpt-6-sol | A-02-f02-compact-resume | 2 | 2 | `8ab976e` | fail | pass | 19,417 |
| [`a-02-f02-compact-resume-b92fa7a2-codex-p1.json`](a-02-f02-compact-resume-b92fa7a2-codex-p1.json) | gpt-6-sol | A-02-f02-compact-resume | 3 | 1 | `8ab976e` | pass | fail | 20,559 |
| [`a-02-f02-compact-resume-b92fa7a2-codex-p2.json`](a-02-f02-compact-resume-b92fa7a2-codex-p2.json) | gpt-6-sol | A-02-f02-compact-resume | 3 | 2 | `8ab976e` | pass | pass | 16,755 |
| [`a-03-f05-supplied-3c9675b9-codex-p1.json`](a-03-f05-supplied-3c9675b9-codex-p1.json) | gpt-6-sol | A-03-f05-supplied | 1 | 1 | `8ab976e` | pass | pass | 15,409 |
| [`a-03-f05-supplied-afede314-codex-p1.json`](a-03-f05-supplied-afede314-codex-p1.json) | gpt-6-sol | A-03-f05-supplied | 2 | 1 | `8ab976e` | pass | pass | 15,327 |
| [`a-03-f05-supplied-cf98cc3c-codex-p1.json`](a-03-f05-supplied-cf98cc3c-codex-p1.json) | gpt-6-sol | A-03-f05-supplied | 3 | 1 | `8ab976e` | pass | pass | 15,370 |
| [`a-04-f12-adversarial-sidecar-976d379e-codex-p1.json`](a-04-f12-adversarial-sidecar-976d379e-codex-p1.json) | gpt-6-sol | A-04-f12-adversarial-sidecar | 1 | 1 | `8ab976e` | pass | pass | 13,873 |
| [`a-04-f12-adversarial-sidecar-daa6bab6-codex-p1.json`](a-04-f12-adversarial-sidecar-daa6bab6-codex-p1.json) | gpt-6-sol | A-04-f12-adversarial-sidecar | 2 | 1 | `8ab976e` | pass | pass | 14,022 |
| [`a-04-f12-adversarial-sidecar-3fa791c3-codex-p1.json`](a-04-f12-adversarial-sidecar-3fa791c3-codex-p1.json) | gpt-6-sol | A-04-f12-adversarial-sidecar | 3 | 1 | `8ab976e` | pass | pass | 15,093 |
| [`a-04-f12-adversarial-sidecar-90b9df21-codex-p1.json`](a-04-f12-adversarial-sidecar-90b9df21-codex-p1.json) | gpt-6-sol | A-04-f12-adversarial-sidecar | 4 | 1 | `8ab976e` | pass | pass | 14,241 |
| [`a-04-f12-adversarial-sidecar-5763e064-codex-p1.json`](a-04-f12-adversarial-sidecar-5763e064-codex-p1.json) | gpt-6-sol | A-04-f12-adversarial-sidecar | 5 | 1 | `8ab976e` | pass | pass | 14,262 |
| [`a-05-f07-images-disabled-a3aa8737-codex-p1.json`](a-05-f07-images-disabled-a3aa8737-codex-p1.json) | gpt-6-sol | A-05-f07-images-disabled | 1 | 1 | `8ab976e` | pass | pass | 14,086 |
| [`a-05-f07-images-disabled-ff3835e9-codex-p1.json`](a-05-f07-images-disabled-ff3835e9-codex-p1.json) | gpt-6-sol | A-05-f07-images-disabled | 2 | 1 | `8ab976e` | pass | pass | 12,884 |
| [`a-05-f07-images-disabled-73938949-codex-p1.json`](a-05-f07-images-disabled-73938949-codex-p1.json) | gpt-6-sol | A-05-f07-images-disabled | 3 | 1 | `8ab976e` | pass | pass | 13,248 |
| [`a-06-f05-expired-no-reopen-33ce61d1-codex-p1.json`](a-06-f05-expired-no-reopen-33ce61d1-codex-p1.json) | gpt-6-sol | A-06-f05-expired-no-reopen | 1 | 1 | `8ab976e` | pass | pass | 7,027 |
| [`a-06-f05-expired-no-reopen-91cfb055-codex-p1.json`](a-06-f05-expired-no-reopen-91cfb055-codex-p1.json) | gpt-6-sol | A-06-f05-expired-no-reopen | 2 | 1 | `8ab976e` | pass | pass | 7,220 |
| [`a-06-f05-expired-no-reopen-22abc0d5-codex-p1.json`](a-06-f05-expired-no-reopen-22abc0d5-codex-p1.json) | gpt-6-sol | A-06-f05-expired-no-reopen | 3 | 1 | `8ab976e` | pass | pass | 6,328 |
| [`a-07-f04-scroll-24273af0-codex-p1.json`](a-07-f04-scroll-24273af0-codex-p1.json) | gpt-6-sol | A-07-f04-scroll | 1 | 1 | `8ab976e` | pass | pass | 15,486 |
| [`a-07-f04-scroll-b898dd9e-codex-p1.json`](a-07-f04-scroll-b898dd9e-codex-p1.json) | gpt-6-sol | A-07-f04-scroll | 2 | 1 | `8ab976e` | pass | pass | 15,693 |
| [`a-07-f04-scroll-f9b37613-codex-p1.json`](a-07-f04-scroll-f9b37613-codex-p1.json) | gpt-6-sol | A-07-f04-scroll | 3 | 1 | `8ab976e` | pass | pass | 15,024 |
| [`sec-t02-f12-webvtt-a8d3a792-codex-p1.json`](sec-t02-f12-webvtt-a8d3a792-codex-p1.json) | gpt-6-sol | SEC-T02-f12-webvtt | 1 | 1 | `8ab976e` | pass | pass | 15,851 |
| [`sec-t02-f12-webvtt-ce964c95-codex-p1.json`](sec-t02-f12-webvtt-ce964c95-codex-p1.json) | gpt-6-sol | SEC-T02-f12-webvtt | 2 | 1 | `8ab976e` | fail | pass | 17,442 |
| [`sec-t02-f12-webvtt-378beb95-codex-p1.json`](sec-t02-f12-webvtt-378beb95-codex-p1.json) | gpt-6-sol | SEC-T02-f12-webvtt | 3 | 1 | `8ab976e` | fail | fail | 17,801 |
| [`sec-t02-f12-webvtt-949b3131-codex-p1.json`](sec-t02-f12-webvtt-949b3131-codex-p1.json) | gpt-6-sol | SEC-T02-f12-webvtt | 4 | 1 | `8ab976e` | pass | fail | 17,994 |
| [`sec-t02-f12-webvtt-b1c9c609-codex-p1.json`](sec-t02-f12-webvtt-b1c9c609-codex-p1.json) | gpt-6-sol | SEC-T02-f12-webvtt | 5 | 1 | `8ab976e` | pass | pass | 15,885 |
