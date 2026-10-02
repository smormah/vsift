'use strict';

// A loopback-only registry for the *local* upgrade mode (RQ-04, `P14 local
// upgrade`): a throwaway Verdaccio on 127.0.0.1 with no uplink, to which the
// job publishes the packages it built, so that a package manager can upgrade a
// real registry's 0.1.0 install to a higher version that exists nowhere else.
//
// It follows the P13 npm qualification driver (npm/qualification/qualify.cjs):
// the registry has no uplink, so a request it receives never reaches another
// registry; the only credential is a throwaway user's token, written to an
// npmrc scoped to the loopback address. Nothing here can publish to a public
// registry: the helpers refuse any registry but the loopback one.

const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const { QualificationError, expect, succeed, writeFile } = require('./common.cjs');

const LOCAL_REGISTRY = 'http://127.0.0.1:4873/';
const LOCAL_HOST = '127.0.0.1:4873';

/** Starts Verdaccio on the loopback address; resolves with the handle `stopRegistry` takes. */
async function startRegistry(verdaccioEntry, work) {
  const root = path.join(work, 'verdaccio');
  fs.mkdirSync(path.join(root, 'storage'), { recursive: true });
  const config = path.join(root, 'config.yaml');
  // No uplinks: a request for any other package is answered here (not found),
  // never passed to a public registry. The audit middleware, which would
  // forward `npm audit` requests to npmjs.com, is off; so is the web UI.
  writeFile(
    config,
    [
      'storage: ./storage',
      'auth:',
      '  htpasswd:',
      '    file: ./htpasswd',
      '    max_users: 1',
      'uplinks: {}',
      'packages:',
      "  'vsift-cli':",
      '    access: $all',
      '    publish: $authenticated',
      "  '@vsift/*':",
      '    access: $all',
      '    publish: $authenticated',
      "  '**':",
      '    access: $all',
      '    publish: $authenticated',
      'middlewares:',
      '  audit:',
      '    enabled: false',
      'web:',
      '  enable: false',
      'log:',
      '  type: stdout',
      '  format: pretty',
      '  level: warn',
      '',
    ].join('\n'),
  );
  const log = fs.openSync(path.join(root, 'verdaccio.log'), 'a');
  const server = childProcess.spawn(process.execPath, [verdaccioEntry, '--config', config, '--listen', LOCAL_HOST], {
    cwd: root,
    stdio: ['ignore', log, log],
    windowsHide: true,
  });
  const deadline = Date.now() + 120_000;
  for (;;) {
    try {
      const response = await fetch(`${LOCAL_REGISTRY}-/ping`);
      if (response.ok) {
        break;
      }
    } catch {
      // Not listening yet.
    }
    expect(server.exitCode === null, `Verdaccio exited ${server.exitCode}; see ${root}/verdaccio.log`);
    expect(Date.now() < deadline, 'Verdaccio did not answer /-/ping within 120 s');
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  return { server, root };
}

async function stopRegistry(registry) {
  if (registry.server.exitCode !== null) {
    return;
  }
  const exited = new Promise((resolve) => registry.server.once('exit', resolve));
  registry.server.kill();
  await exited;
}

/** A throwaway user of the loopback registry; returns its token. */
async function registryToken() {
  const name = 'vsift-qualification';
  const response = await fetch(`${LOCAL_REGISTRY}-/user/org.couchdb.user:${name}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name, password: crypto.randomBytes(24).toString('hex'), type: 'user', roles: [] }),
  });
  const body = await response.json();
  expect(response.status === 201 && typeof body.token === 'string', `the registry refused the user: ${response.status}`);
  return body.token;
}

/**
 * Publishes the packed tarballs to the loopback registry under `tag`, platform
 * packages first. The registry is the only place `npm publish` is told to go.
 */
async function publishLocal(tarballs, tag, env, work) {
  expect(tarballs.length === 4, `expected four tarballs, found ${tarballs.length}`);
  const token = await registryToken();
  const config = path.join(work, 'publish.npmrc');
  // Scoped to the loopback registry: npm sends this token nowhere else.
  writeFile(config, `registry=${LOCAL_REGISTRY}\n//${LOCAL_HOST}/:_authToken=${token}\n`);
  const ordered = [...tarballs].sort((left, right) => Number(path.basename(left).startsWith('vsift-cli-')) - Number(path.basename(right).startsWith('vsift-cli-')));
  try {
    for (const tarball of ordered) {
      succeed('npm', ['publish', tarball, '--tag', tag, '--ignore-scripts', '--registry', LOCAL_REGISTRY, '--userconfig', config], env);
    }
  } finally {
    fs.rmSync(config, { force: true });
  }
  return ordered.map((file) => path.basename(file));
}

/** Refuses any registry but the loopback one, for code that is about to talk to one. */
function requireLoopback(registry) {
  if (registry !== LOCAL_REGISTRY) {
    throw new QualificationError(`${registry} is not the loopback registry ${LOCAL_REGISTRY}`);
  }
}

module.exports = { LOCAL_HOST, LOCAL_REGISTRY, publishLocal, registryToken, requireLoopback, startRegistry, stopRegistry };
