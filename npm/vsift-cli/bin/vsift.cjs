#!/usr/bin/env node
// The `vsift` command: runs the native vsift executable for this machine
// (see ../lib/launcher.cjs). It always runs, however it is loaded: Bun and
// package-manager shims may load it through a wrapper of their own.

'use strict';

require('../lib/launcher.cjs').main();
