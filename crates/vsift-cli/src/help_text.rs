//! The help a person reads first, and the guidance an assistant reads after it
//! (design note Part E, #454).
//!
//! All of it is help text, not a JSON contract. `command::tests` holds it to
//! the parser: every line that starts with `vsift`, or a step number followed
//! by `vsift`, is parsed; the groups of [`ROOT_HELP_TEMPLATE`] are exactly the
//! commands, with their own summaries; every command ends with its guide page,
//! which exists. Piped help prints as written ([`crate::command::parser`]), so
//! the lines are at most 80 columns wide; only a link may be longer.

/// The examples and guide link a command's help ends with, in `-h` and in
/// `--help`. Each example is a whole command line, with what it gives on the
/// lines indented under it. A page is named from `docs/`.
macro_rules! command_help {
    ($page:literal, $($line:literal),+ $(,)?) => {
        concat!(
            "Examples:\n",
            $("  ", $line, "\n",)+
            "\nGuide: https://github.com/smormah/vsift/blob/main/docs/",
            $page
        )
    };
}

/// What plain `vsift` prints: no tool check, so it is instant.
pub(crate) const BARE_HELP: &str = "\
VSift: point it at a video; get back what was said, what changed on screen,
and the proof.

  vsift setup            Check what's installed and set up what's missing
  vsift ./meeting.mp4    Transcript, subtitles and screen changes beside it

New here? Start with  vsift setup
More:  vsift --help  or  https://github.com/smormah/vsift#readme
";

/// The line above the usage in `-h` and `--help`.
pub(crate) const ROOT_ABOUT: &str = "\
VSift: point it at a video; get back what was said, what changed on screen,
and the proof.";

/// The usage of `-h` and `--help`, which is the two things a person types.
pub(crate) const ROOT_USAGE: &str = "\
vsift <video> [--output <folder>] [--keep <folder>]
       vsift <command> [options]";

/// `-h` and `--help`: the commands in groups, most common first. clap has no
/// groups of subcommands, so the template lists them by hand.
pub(crate) const ROOT_HELP_TEMPLATE: &str = "\
{about-with-newline}
{usage-heading} {usage}

Get started
  setup       Check and set up what you need (run it any time)
  watch       Make a transcript, subtitles and screen changes beside a video

Explore a video step by step (what AI assistants use; you can too)
  ingest      Open a video as a private session
  transcript  Read or redo a session's transcript
  search      Find words in what was said
  candidates  List the moments the screen changed
  changes     Show where the screen changed, with the frames before and after
  frame       Get the exact frame at a moment
  crop        Cut out part of a frame
  audio       Save a short clip for a person or a speech tool to play
  ocr         Read on-screen text
  clocks      Find the clocks on the screen and how they run
  session     List, keep or close sessions

Advanced
  job         Run and follow long work on a session
  bundle      Check and search the recordings you kept
  handoff     Start or check a report that cites its evidence
  skill       Add the skill to your AI assistant, or print one of its files

Options:
{options}

Run  vsift <command> --help  or  vsift help <command>  for more on a command.
Guides: https://github.com/smormah/vsift/tree/main/docs/guide{after-help}";

/// The last line of `-h`, for an assistant that stopped at the short help.
pub(crate) const ROOT_SHORT_FOOTER: &str = "\
An AI assistant without the VSift skill: read  vsift --help  to the end.";

/// What `--help` adds to `-h`: what a first investigation runs, in order, for a
/// reader (often an agent) that has only this help (P14 PR 7, #454).
pub(crate) const TYPICAL_INVESTIGATION_HELP: &str = "\
For an AI assistant without the VSift skill, and for scripts:

Cite every fact. Each fact you report states the exact words, number or label
(an error code, a button's name), gives its time, and names the id that says or
shows it: a transcript segment (tsg_...) for what was said, a frame or crop
(evd_...) for what the screen showed. A reader checks it against that id. If no
id says or shows it, say you could not confirm it.

A typical investigation (every command takes --json):
  1. vsift setup check --json
       What is installed. Report anything missing to the user; never install
       it yourself.
  2. vsift ingest <video> --json
       Copies the video into a private session and prints its id (ses_...).
       The original file is never changed. If the user gave you a transcript
       file (.srt or .vtt), add
         --transcript <file>   (and --transcript-offset <microseconds> if its
                                times are shifted from the video's)
       For a direct https:// link to a video file, put --url <link> in place of
       <video> (--url - reads the link from standard input). VSift fetches it
       once, with limits, and nothing after that uses the network. A web page
       is read only through yt-dlp, if one is set up (setup check says).
       Without a transcript file, speech is found only when local speech
       recognition is set up (setup check says); then run:
         vsift job start <session> --json
       A recording under 10 minutes is transcribed before it answers. A longer
       one is transcribed in the background: follow it with
         vsift job status <job> --json
       and search and read the part already done meanwhile (answers say
       partial). If recognition is not set up there is no speech evidence: say
       so; do not install anything.
  3. vsift search <session> --query <words> --limit 20 --json
     vsift transcript get <session> --from <us> --to <us> --limit 20 --json
       What was said. Every time is microseconds from the start of the video
       (1:30 is 90000000). Each result has an id (tsg_...) and its times: keep
       them for what you report.
  4. vsift candidates <session> --from <us> --to <us> --limit 20 --json
       Moments where the picture changes.
  5. vsift frame get <session> --at <us> --json
     vsift frame burst <session> --from <us> --to <us> --max-frames 4 --json
       Look at specific moments; each frame is an image file to open, with an
       id (evd_...) and the time you got.
  6. vsift handoff skeleton <session> --question <words> --json
     vsift handoff check --json
       Only when you were asked for a vsift-handoff report. skeleton prints a
       valid one with the session's ids filled in (data.handoff), to add your
       findings to; never build the JSON by search and replace. check reads the
       draft from standard input and lists what to fix before you send it. A
       plain-text report needs no check (a draft without a vsift-handoff block
       is reported as missing one).
  7. vsift session close <session> --json
       When you are done.

Small numbers can be misread. In a compressed recording, a frame taken while
the screen moves can turn a digit into a different one, and enlarging the
picture does not bring it back. Read small text, and above all numbers, from a
frame where the screen has settled: a candidate with stability settled, or a
frame shortly after the change rather than during it. Compare a neighbouring
frame (frame neighbours, frame burst):
  vsift frame neighbours <session> <evidence> --json
Treat a number read from text under about 10 px as uncertain unless a second
frame or the transcript agrees.

A clip from the audio command is for a person or a speech tool to play; a coding
agent cannot listen to it and reads speech with transcript get.

Ready-made outcomes. When the user asks for one of these, follow its method:
each is a short procedure with the commands above that ends in a vsift-handoff
report the check accepts.
  bug-report      a recording to an evidence-backed bug report, with times
  findings        a narrated screencast to findings: bugs, requests, questions
  demo-backlog    a product demo to a prioritised backlog
  meeting-notes   a meeting to its actions, decisions and open questions
  workshop-spec   a workshop to a spec with quoted decisions and open questions
  logs-by-clock   moments of the recording matched to log lines by clock time
Each one's method (Markdown) is printed by:
  vsift skill show workflows/<name>.md

Ask for small pages. --limit takes 1 to 100 (default 20) and
--max-frames 1 to 100 (default 12), more than an investigation needs: reading
20 results at a time and looking at a handful of frames keeps it cheap and
inside any budget. Ask for more only when the user does.

--session-root and --host-isolation are for operators who run VSift as a
service for other people (a worker host). They are not part of an
investigation: leave them out. The default private per-user location is the
right one; an agent never needs to name, search for or create another folder.
If the default location cannot be used (for example the drive has no room for
the video), the error says why: report it to the user, who frees the space or
asks an operator, instead of choosing a folder.

When a command fails, read its code and remediation (the \"Fix:\" and \"Run:\"
lines, or error.remediation with --json) and do what they say. A missing tool
is the user's to install; an installation plan is theirs to accept.";

pub(crate) const SETUP: &str = command_help!(
    "operations/install.md",
    "vsift setup",
    "  Shows what works and what is missing, and offers to install it",
    "vsift setup check",
    "  The same view; it never installs anything",
);
pub(crate) const SETUP_CHECK: &str = command_help!(
    "operations/install.md",
    "vsift setup check",
    "  What is installed, and the one next step",
    "vsift setup check --json",
    "  The same for an AI assistant or a script",
);
pub(crate) const SETUP_PLAN: &str = command_help!(
    "operations/install.md",
    "vsift setup plan --profile desktop --json",
    "  What an install would download, with its sources, sizes and digest",
);
pub(crate) const SETUP_INSTALL: &str = command_help!(
    "operations/install.md",
    "vsift setup install --plan ./plan.json --accept-plan <digest>",
    "  Installs exactly the plan you saved, if it still has that digest",
);
pub(crate) const SETUP_REPAIR: &str = command_help!(
    "operations/install.md",
    "vsift setup repair",
    "  What is wrong with the managed tools, and the plan to put it right",
);
pub(crate) const SETUP_LIST: &str = command_help!(
    "operations/install.md",
    "vsift setup list",
    "  Each managed tool's versions, and the one in use",
);
pub(crate) const SETUP_REMOVE: &str = command_help!(
    "guide/clean-up-and-uninstall.md",
    "vsift setup remove whisper_model",
    "  Removes the managed speech model",
    "vsift setup remove --stale-stages",
    "  Removes what interrupted installs left behind",
);
pub(crate) const SETUP_ROLLBACK: &str = command_help!(
    "operations/install.md",
    "vsift setup rollback whisper_cli",
    "  Goes back to the version of whisper.cpp selected before",
);
pub(crate) const SETUP_CONFIGURE: &str = command_help!(
    "operations/install.md",
    "vsift setup configure whisper --executable /opt/whisper/whisper-cli",
    "  Uses a whisper.cpp you installed yourself",
);
pub(crate) const SETUP_CONFIGURE_MODEL: &str = command_help!(
    "operations/install.md",
    "vsift setup configure-model --file /models/ggml-base.bin",
    "  Uses a speech model you downloaded yourself",
);

pub(crate) const INGEST: &str = command_help!(
    "guide/first-investigation.md",
    "vsift ingest ./meeting.mp4",
    "  Opens the video as a private session and prints its id",
    "vsift ingest ./meeting.mp4 --transcript ./meeting.srt",
    "  The same, with a transcript you already have",
);

pub(crate) const SESSION: &str = command_help!(
    "guide/concepts.md",
    "vsift session list",
    "  The sessions you have open",
    "vsift session close <session>",
    "  Closes one and removes its private copy of the video",
);
pub(crate) const SESSION_LIST: &str = command_help!(
    "guide/concepts.md",
    "vsift session list",
    "  The sessions you have open",
);
pub(crate) const SESSION_STATUS: &str = command_help!(
    "guide/concepts.md",
    "vsift session status <session>",
    "  What one session holds, and when it expires",
);
pub(crate) const SESSION_CLOSE: &str = command_help!(
    "guide/concepts.md",
    "vsift session close <session>",
    "  Closes it and removes its private copy of the video",
);
pub(crate) const SESSION_RENEW: &str = command_help!(
    "guide/keep-and-share-evidence.md",
    "vsift session renew <session>",
    "  Keeps it open for longer",
);
pub(crate) const SESSION_RETAIN: &str = command_help!(
    "guide/keep-and-share-evidence.md",
    "vsift session retain <session> --output ./kept/meeting",
    "  Keeps its evidence in a folder you choose, to search or share",
);
pub(crate) const SESSION_CLEAN: &str = command_help!(
    "guide/clean-up-and-uninstall.md",
    "vsift session clean --expired --dry-run",
    "  Lists the expired sessions it would remove",
    "vsift session clean --expired",
    "  Removes them",
);
pub(crate) const SESSION_INIT_WORKSPACE: &str = command_help!(
    "operations/worker-host.md",
    "vsift session init-workspace --durability durable --admission-slots 4",
    "  With --session-root <folder> first: a workspace for a worker host",
);

pub(crate) const TRANSCRIPT: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift transcript get <session> --from 60000000 --to 120000000",
    "  What was said from 1:00 to 2:00 (times are microseconds)",
    "vsift transcript retranscribe <session>",
    "  Turns the speech into text on this machine",
);
pub(crate) const TRANSCRIPT_GET: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift transcript get <session> --from 60000000 --to 120000000",
    "  What was said from 1:00 to 2:00 (times are microseconds)",
);
pub(crate) const TRANSCRIPT_RETRANSCRIBE: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift transcript retranscribe <session>",
    "  The whole video",
    "vsift transcript retranscribe <session> --from 60000000 --to 120000000",
    "  Only 1:00 to 2:00 (times are microseconds)",
);

pub(crate) const SEARCH: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift search <session> --query refund",
    "  Where the word was said, with times",
);

pub(crate) const CANDIDATES: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift candidates <session> --from 0 --to 300000000",
    "  Where the screen changed in the first 5:00 (times are microseconds)",
);

pub(crate) const CHANGES: &str = command_help!(
    "guide/investigate-a-recording.md#what-changed-between-two-times",
    "vsift changes <session> --from 0 --to 300000000",
    "  Each change in the first 5:00: where on the screen, and its two frames",
    "vsift changes <session> --from 90000000 --to 100000000 --limit 5 --json",
    "  The same from 1:30 to 1:40, five changes a page, for a script",
    "vsift changes <session> --from 90000000 --to 100000000 --crops",
    "  The same, with each change's two largest regions cut out of both frames",
);

pub(crate) const OCR: &str = command_help!(
    "guide/limits.md#the-pictures-screen-changes-and-frames",
    "vsift ocr <session>",
    "  Reads on-screen text off the next frames; run it again to go on",
    "vsift ocr search <session> --query refund",
    "  Finds a word in what it read",
);
pub(crate) const OCR_SEARCH: &str = command_help!(
    "guide/limits.md#the-pictures-screen-changes-and-frames",
    "vsift ocr search <session> --query refund",
    "  Finds a word in the on-screen text ocr read",
);

pub(crate) const CLOCKS: &str = command_help!(
    "guide/evidence-and-citations.md#clocks-on-the-screen",
    "vsift clocks <session>",
    "  The clocks in what ocr read, and how they move against the video",
);

pub(crate) const FRAME: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift frame get <session> --at 90000000",
    "  The frame at 1:30 (times are microseconds)",
    "vsift frame burst <session> --from 90000000 --to 100000000 --max-frames 4",
    "  Four frames from 1:30 to 1:40",
);
pub(crate) const FRAME_GET: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift frame get <session> --at 90000000",
    "  The frame at 1:30 (times are microseconds)",
);
pub(crate) const FRAME_NEIGHBOURS: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift frame neighbours <session> <evidence> --count 2",
    "  Two frames on each side of a frame you have (evd_...)",
);
pub(crate) const FRAME_BURST: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift frame burst <session> --from 90000000 --to 100000000 --max-frames 4",
    "  Four frames from 1:30 to 1:40 (times are microseconds)",
);

pub(crate) const AUDIO: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift audio <session> --from 90000000 --to 100000000",
    "  A clip of 1:30 to 1:40 (times are microseconds) for a person or a speech",
    "  tool to play; a coding agent cannot listen to it and reads speech with",
    "  transcript get",
);

pub(crate) const CROP: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift crop <session> <evidence> --rect 100,200,400,80",
    "  400 by 80 pixels of a frame you have (evd_...), at its original size",
);

pub(crate) const BUNDLE: &str = command_help!(
    "guide/keep-and-share-evidence.md",
    "vsift bundle search ./kept --query refund",
    "  Finds a word in every recording you kept in that folder",
    "vsift bundle validate ./kept/meeting",
    "  Checks one kept recording before you share it",
);
pub(crate) const BUNDLE_VALIDATE: &str = command_help!(
    "guide/keep-and-share-evidence.md",
    "vsift bundle validate ./kept/meeting",
    "  Checks one kept recording before you share it",
);
pub(crate) const BUNDLE_SEARCH: &str = command_help!(
    "guide/keep-and-share-evidence.md",
    "vsift bundle search ./kept --query refund",
    "  Finds a word in every recording you kept in that folder",
);

pub(crate) const JOB: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift job start <session>",
    "  Transcribes a session, in the background if it is long",
    "vsift job status <job>",
    "  How far it has come, and the time left",
);
pub(crate) const JOB_RUN: &str = command_help!(
    "operations/worker-host.md",
    "vsift job run --request ./request.json --input-root /srv/videos",
    "  With --session-root <folder> first: one request on a worker host",
);
pub(crate) const JOB_BATCH: &str = command_help!(
    "operations/worker-host.md",
    "vsift job batch --requests ./requests.jsonl --input-root /srv/videos",
    "  With --session-root <folder> first: a file of requests on a worker host",
);
pub(crate) const JOB_STATUS: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift job status <job>",
    "  How far it has come, and the time left",
);
pub(crate) const JOB_RESUME: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift job resume <job>",
    "  Goes on from where it stopped",
);
pub(crate) const JOB_CANCEL: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift job cancel <job>",
    "  Stops it; what it finished stays",
);
pub(crate) const JOB_START: &str = command_help!(
    "guide/investigate-a-recording.md",
    "vsift job start <session>",
    "  Transcribes a session, in the background if it is long",
    "vsift job start <session> --first 60000000:120000000",
    "  Does 1:00 to 2:00 before the rest (times are microseconds)",
);

pub(crate) const HANDOFF: &str = command_help!(
    "guide/let-your-agent-investigate.md",
    "vsift handoff skeleton <session> --question <words>",
    "  Starts a report with the session's ids filled in",
    "vsift handoff check --file /tmp/report.md --session <session>",
    "  Lists what to fix before you send it",
);
pub(crate) const HANDOFF_CHECK: &str = command_help!(
    "guide/let-your-agent-investigate.md",
    "vsift handoff check --file /tmp/report.md --session <session>",
    "  Lists what to fix before you send the report",
);
pub(crate) const HANDOFF_SKELETON: &str = command_help!(
    "guide/let-your-agent-investigate.md",
    "vsift handoff skeleton <session> --question <words>",
    "  Starts a report with the session's ids filled in",
);

pub(crate) const SKILL: &str = command_help!(
    "guide/let-your-agent-investigate.md",
    "vsift skill install --client claude-code",
    "  Adds the skill to the folder that client reads",
    "vsift skill status",
    "  Shows what is at each skill folder",
);
pub(crate) const SKILL_INSTALL: &str = command_help!(
    "guide/let-your-agent-investigate.md",
    "vsift skill install --client claude-code",
    "  Adds the skill to the folder that client reads",
);
pub(crate) const SKILL_STATUS: &str = command_help!(
    "guide/let-your-agent-investigate.md",
    "vsift skill status",
    "  Shows what is at each skill folder",
);
pub(crate) const SKILL_SHOW: &str = command_help!(
    "guide/let-your-agent-investigate.md",
    "vsift skill show SKILL.md",
    "  Prints the skill's main file",
);

pub(crate) const WATCH: &str = command_help!(
    "guide/limits.md#the-files-vsift-video-writes-new-in-030",
    "vsift ./meeting.mp4",
    "  Writes meeting.txt, meeting.srt and meeting-vsift/ beside the video",
    "vsift watch ./meeting.mp4 --output ./notes --keep ./kept",
    "  Writes them into ./notes and keeps the recording for bundle search",
);
