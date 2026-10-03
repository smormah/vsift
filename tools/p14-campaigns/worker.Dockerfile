# The worker image of the P14 campaigns (RQ-09 load, RQ-10 malicious media,
# RQ-12 the runbook walk): the pinned Ubuntu 24.04 image that ci.yml and the
# P14 published-artifact workflows already use; the one library the reviewed
# whisper.cpp build needs and a minimal image lacks (libgomp1, issue #256);
# the CA bundle the managed install needs to download from the publishers; an
# unprivileged `vsift` account (uid 10001, as docs/operations/worker-host.md
# section 10 uses); and the published `vsift` executable, copied from the
# installed npm package, never built from source. It is a disposable image of
# a hosted runner, built for each run.
#
# The tools are not in the image: the managed install puts them in the
# worker's per-user folder (a volume), as a user's `setup install` would. The
# Ubuntu packages are the archive's current versions when the image is built
# (not pinned: the image is rebuilt for every run and says so in the job).
FROM ubuntu@sha256:a61567bd31828687156d735ea8eb01ba4e37636e225dd6a48ba94136a70d9d61

RUN apt-get update \
 && apt-get install --no-install-recommends --yes libgomp1 ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 10001 --user-group --no-create-home --shell /usr/sbin/nologin vsift

COPY --chmod=0755 vsift /usr/local/bin/vsift
COPY --chmod=0755 case-wrapper.sh /usr/local/bin/case-wrapper

USER 10001:10001
