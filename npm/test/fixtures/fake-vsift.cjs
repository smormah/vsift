'use strict';

// Stands in for the native executable in the launcher tests. The tests copy
// the Node.js executable into a fake platform package, so the launcher runs
// `node fake-vsift.cjs <mode> ...`; the mode says what to do.

const [mode, ...rest] = process.argv.slice(2);

function waitForever() {
  setInterval(() => {}, 60_000);
}

switch (mode) {
  case 'args':
    process.stdout.write(JSON.stringify(rest));
    break;
  case 'exit':
    process.exitCode = Number(rest[0]);
    break;
  case 'streams': {
    const chunks = [];
    process.stdin.on('data', (chunk) => chunks.push(chunk));
    process.stdin.on('end', () => {
      process.stdout.write(Buffer.concat(chunks));
      process.stderr.write('to stderr');
    });
    break;
  }
  case 'trap': {
    // Like a long vsift command: the first interruption cancels, and the
    // command then ends with CANCELLED (exit 6) after a short cleanup.
    const seen = [];
    const onSignal = (signal) => {
      seen.push(signal);
      if (seen.length === 1) {
        setTimeout(() => {
          process.stdout.write(`caught ${seen.join(',')}\n`);
          process.exit(6);
        }, 300);
      }
    };
    for (const signal of process.platform === 'win32' ? ['SIGINT', 'SIGBREAK'] : ['SIGINT', 'SIGTERM']) {
      process.on(signal, () => onSignal(signal));
    }
    process.stdout.write('ready\n');
    waitForever();
    break;
  }
  case 'hang':
    // Like a short vsift command: the operating system's default ends it.
    process.stdout.write('ready\n');
    waitForever();
    break;
  default:
    process.stderr.write(`unknown mode ${mode}\n`);
    process.exitCode = 99;
}
