# Security policy

VSift processes untrusted media and invokes specialist native tools. Security reports are taken seriously.

## Supported versions

VSift has not published a stable release. Until then, security fixes are applied to the default branch only. A supported-version table will be introduced with the first stable release.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability.

Use the repository's **Security** tab and select **Report a vulnerability** to submit a private GitHub security advisory. Include:

- affected revision or version;
- operating system and architecture;
- reproduction steps or a minimal synthetic fixture;
- potential impact;
- any suggested mitigation.

Do not send sensitive real-world recordings as evidence. Construct the smallest rights-safe reproduction possible.

The maintainers will acknowledge a complete report as soon as practical, assess severity, coordinate remediation and disclosure, and credit reporters who wish to be named.

## Security scope

High-priority areas include:

- command or argument injection;
- unsafe archive extraction or path traversal;
- deletion outside VSift-owned temporary storage;
- unverified binary or model downloads;
- sensitive transcript, path, or credential disclosure;
- denial of service through unbounded media or provider output;
- malicious media exploiting VSift's own parsing or orchestration.

Vulnerabilities in FFmpeg, Whisper, operating-system components, or other upstream software should also be reported to the relevant upstream project. Reports are still welcome when VSift can reduce exposure or improve isolation.

