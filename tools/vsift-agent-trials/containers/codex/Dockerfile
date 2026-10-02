# syntax=docker/dockerfile:1
#
# The Linux environment for P12's named Codex trials (ADR 0022, 2026-09-28
# note; runbook docs/agents/trials.md, "Codex trials in a Linux container").
#
# Two images, built from the repository root at the commit under test
# (tools/vsift-agent-trials/containers/codex/codex-trial.ps1 build does both):
#
#   docker build --file tools/vsift-agent-trials/containers/codex/Dockerfile \
#     --build-arg VSIFT_COMMIT=<git rev-parse HEAD> --target agent \
#     --tag vsift-codex-trials-agent:<short sha> .
#   (the same with --target harness --tag vsift-codex-trials-harness:<short sha>)
#
# `agent` is where Codex and its agent run: the built tools only, none of
# the repository. `harness` adds the repository for `prepare` and `grade`,
# which run as separate containers before and after the agent.
#
# P14 adds two more targets, `agent-published` and `harness-published`, for the
# clean-install and cold-agent trials: the same Codex, but VSift is installed
# from the REAL npm registry at build time (`npm install --global
# vsift-cli@<exact version>` into /opt/vsift-published, by the harness's own
# `install` step, which records the registry integrity and the launcher's
# digest check in /opt/vsift-published-proof.json), never copied from a
# repository build:
#
#   docker build --file tools/vsift-agent-trials/containers/codex/Dockerfile \
#     --build-arg VSIFT_COMMIT=<the commit under test> \
#     --build-arg VSIFT_PUBLISHED_VERSION=<exact version, for example 0.1.0> \
#     --target agent-published --tag vsift-codex-trials-agent-published:<tag> .
#   (the same with --target harness-published)
#
# The published agent image has no FFmpeg, no whisper.cpp and no model: the
# harness installs the managed tools into each trial with `setup install`, as
# a user on Ubuntu does. FFmpeg exists only in the published harness image, to
# build the loop and blur clips; the agent never runs in it.
#
# Every input is pinned and verified; nothing is taken from a moving tag:
#   - Ubuntu 24.04, by image digest (the digest CI's strict-worker job uses);
#   - Rust, by the toolchain in rust-toolchain.toml, installed by a rustup-init
#     whose SHA-256 is pinned below; crates from Cargo.lock (--locked);
#   - FFmpeg and FFprobe 9.0.1, the reviewed BtbN build CI's P07 local-ASR
#     workflow uses (tools/p06_ubuntu_candidate_smoke.py: archive size and
#     SHA-256, then each selected file's size and SHA-256);
#   - whisper.cpp v1.9.2, built from the tag commit the P06 review pinned
#     (docs/planning/p06-ubuntu-artifact-candidate.md);
#   - codex-cli 0.155.0-alpha.16, the official openai/codex Linux package, by
#     size and the SHA-256 GitHub publishes for the release asset.
#   - Node.js 24.21.0 (the published images only), the linux-x64 tarball, by the
#     SHA-256 nodejs.org publishes in SHASUMS256.txt (the version the release
#     workflow publishes with).
# The model is not in the image: the operator mounts the reviewed
# ggml-base.bin read-only and the trial driver checks its digest.
#
# No credential is ever in the image: the Codex sign-in is mounted read-only
# at run time and copied into a per-run CODEX_HOME on a tmpfs.

ARG UBUNTU_IMAGE=ubuntu@sha256:a61567bd31828687156d735ea8eb01ba4e37636e225dd6a48ba94136a70d9d61

# ---------------------------------------------------------------------------
# Build tools from Ubuntu's signed archive. The base image has no CA
# certificates, so apt's first step fetches only ca-certificates over the
# image's default HTTP source (apt checks every package against the archive's
# GPG-signed metadata either way); every later apt fetch uses HTTPS.
FROM ${UBUNTU_IMAGE} AS build-base
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install --yes --no-install-recommends ca-certificates \
 && sed -i 's#http://#https://#g' /etc/apt/sources.list.d/ubuntu.sources \
 && apt-get update \
 && apt-get install --yes --no-install-recommends \
      build-essential cmake curl git libssl-dev pkg-config xz-utils \
 && rm -rf /var/lib/apt/lists/*

# ---------------------------------------------------------------------------
# VSift and the trial harness, from the build context (the commit under test).
FROM build-base AS vsift-build
# rustup-init 1.29.1 and the SHA-256 static.rust-lang.org publishes beside it
# (rustup-init.sha256), checked again on download on 2026-09-29.
ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_INIT_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
ENV RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo PATH=/opt/rust/cargo/bin:${PATH}
RUN curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
      --max-filesize 30000000 \
      --output /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
 && echo "${RUSTUP_INIT_SHA256}  /tmp/rustup-init" | sha256sum --check --strict \
 && chmod 0755 /tmp/rustup-init \
 && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none \
 && rm /tmp/rustup-init
# The source is built where the harness's compiled-in default repository
# points (CARGO_MANIFEST_DIR/../..), the path the `harness` image copies it to.
WORKDIR /opt/vsift/src
# rustup installs the toolchain rust-toolchain.toml names (1.98.1) and checks
# each component against the channel manifest's hashes.
COPY rust-toolchain.toml ./
RUN rustup toolchain install && rustc --version
COPY . .
RUN --mount=type=cache,target=/opt/rust/cargo/registry \
    --mount=type=cache,target=/opt/vsift/src/target \
    cargo build --release --locked -p vsift-cli -p vsift-agent-trials \
 && install -D -m 0755 target/release/vsift /out/opt/vsift/bin/vsift \
 && install -D -m 0755 target/release/vsift-agent-trials /out/opt/vsift-trials/bin/vsift-agent-trials

# ---------------------------------------------------------------------------
# whisper.cpp v1.9.2 from the pinned tag commit.
FROM build-base AS whisper-build
ARG WHISPER_TAG=v1.9.2
ARG WHISPER_COMMIT=306c88f4d1286aec1bf96e544632897886af5501
RUN git clone --quiet --depth 1 --branch "${WHISPER_TAG}" \
      https://github.com/ggml-org/whisper.cpp /build/whisper.cpp \
 && test "$(git -C /build/whisper.cpp rev-parse HEAD)" = "${WHISPER_COMMIT}"
# A static, portable CPU build: no host-specific instructions beyond the
# Haswell baseline (AVX2, FMA, F16C), no OpenMP, SDL or network support.
RUN cmake -S /build/whisper.cpp -B /build/out \
      -DCMAKE_BUILD_TYPE=Release \
      -DBUILD_SHARED_LIBS=OFF \
      -DGGML_NATIVE=OFF -DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON \
      -DGGML_OPENMP=OFF \
      -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF \
      -DWHISPER_SDL2=OFF -DWHISPER_CURL=OFF \
 && cmake --build /build/out --target whisper-cli --parallel \
 && install -D -m 0755 /build/out/bin/whisper-cli /out/opt/whisper.cpp/bin/whisper-cli \
 && install -D -m 0644 /build/whisper.cpp/LICENSE /out/opt/whisper.cpp/LICENSE

# ---------------------------------------------------------------------------
# FFmpeg and FFprobe: the reviewed BtbN build, archive and files verified.
FROM build-base AS ffmpeg-fetch
ARG FFMPEG_URL=https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-31-13-27/ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0.tar.xz
ARG FFMPEG_SIZE=113372924
ARG FFMPEG_SHA256=204fc02692b11249c3e688ad18538ce2939129a1fc6abc32a6b2638a024496cf
ARG FFMPEG_ROOT=ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0
RUN curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
      --max-filesize 120000000 --output /tmp/ffmpeg.tar.xz "${FFMPEG_URL}" \
 && test "$(stat -c %s /tmp/ffmpeg.tar.xz)" = "${FFMPEG_SIZE}" \
 && echo "${FFMPEG_SHA256}  /tmp/ffmpeg.tar.xz" | sha256sum --check --strict \
 && mkdir -p /tmp/ffmpeg \
 && tar -xJf /tmp/ffmpeg.tar.xz -C /tmp/ffmpeg --no-same-owner \
      "${FFMPEG_ROOT}/bin/ffmpeg" "${FFMPEG_ROOT}/bin/ffprobe" "${FFMPEG_ROOT}/LICENSE.txt" \
 && cd "/tmp/ffmpeg/${FFMPEG_ROOT}" \
 && printf '%s\n' \
      "ed57193f048a65bfb0aa3c360639d7f7109ca014405201e3ea478c9ca4ea20fc  bin/ffmpeg" \
      "0e3357bef1737ec02ae600e7f6e4e409966d8d0647521ca523c622be574137b7  bin/ffprobe" \
      "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768  LICENSE.txt" \
    | sha256sum --check --strict \
 && install -D -m 0755 bin/ffmpeg /out/opt/ffmpeg/bin/ffmpeg \
 && install -D -m 0755 bin/ffprobe /out/opt/ffmpeg/bin/ffprobe \
 && install -D -m 0644 LICENSE.txt /out/opt/ffmpeg/LICENSE.txt \
 && rm -rf /tmp/ffmpeg /tmp/ffmpeg.tar.xz

# ---------------------------------------------------------------------------
# codex-cli: the official Linux package of the Windows client's release. It
# carries Codex's own bubblewrap (codex-resources/bwrap), which Codex checks
# by digest before use.
FROM build-base AS codex-fetch
ARG CODEX_VERSION=0.155.0-alpha.16
ARG CODEX_URL=https://github.com/openai/codex/releases/download/rust-v0.155.0-alpha.16/codex-package-x86_64-unknown-linux-musl.tar.gz
ARG CODEX_SIZE=144704074
ARG CODEX_SHA256=1fc1c6284cb3425b806e75209f6272151ac8632a86deb342b62f8c13f98f18c0
RUN curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
      --max-filesize 150000000 --output /tmp/codex.tar.gz "${CODEX_URL}" \
 && test "$(stat -c %s /tmp/codex.tar.gz)" = "${CODEX_SIZE}" \
 && echo "${CODEX_SHA256}  /tmp/codex.tar.gz" | sha256sum --check --strict \
 && mkdir -p /out/opt/codex \
 && tar -xzf /tmp/codex.tar.gz -C /out/opt/codex --no-same-owner \
 && grep -q "\"version\": \"${CODEX_VERSION}\"" /out/opt/codex/codex-package.json \
 && test -x /out/opt/codex/bin/codex && test -x /out/opt/codex/codex-resources/bwrap \
 && rm /tmp/codex.tar.gz

# ---------------------------------------------------------------------------
# `agent`: the image the Codex client and its agent run in. It holds the
# built tools and nothing of the repository: no corpus, no truth, no
# scenarios, no tests, no skill source. `prepare` (in the `harness` image)
# has already copied the skill, the video and any sidecar into the trial
# workspace.
FROM ${UBUNTU_IMAGE} AS agent
ARG VSIFT_COMMIT=unknown
ENV DEBIAN_FRONTEND=noninteractive
# ca-certificates for Codex's model API, libssl for vsift, curl for the
# sandbox's network check; fetched over HTTPS with the build stage's CA
# bundle.
COPY --from=build-base /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN sed -i 's#http://#https://#g' /etc/apt/sources.list.d/ubuntu.sources \
 && apt-get update \
 && apt-get install --yes --no-install-recommends ca-certificates curl libssl3t64 \
 && rm -rf /var/lib/apt/lists/*
# The trial operator: a fixed, unprivileged account. USER and LOGNAME stay
# unset, so the harness's report checks name no account.
RUN groupadd --gid 10001 trials \
 && useradd --uid 10001 --gid 10001 --home-dir /home/trials --create-home --shell /bin/bash trials \
 && install -d -o 10001 -g 10001 -m 0700 /trials /exports
COPY --from=vsift-build /out/ /
COPY --from=whisper-build /out/ /
COPY --from=ffmpeg-fetch /out/ /
COPY --from=codex-fetch /out/ /
COPY --chmod=0755 tools/vsift-agent-trials/containers/codex/trial-driver.sh /opt/vsift-trials/bin/trial-driver
RUN /opt/vsift/bin/vsift --version \
 && /opt/codex/bin/codex --version \
 && /opt/ffmpeg/bin/ffmpeg -hide_banner -version | head -n 1 \
 && /opt/whisper.cpp/bin/whisper-cli --help > /dev/null 2>&1 \
 && test ! -e /opt/vsift/src
LABEL org.opencontainers.image.title="vsift-codex-trials-agent" \
      org.opencontainers.image.description="P12 Codex trials: the image the agent runs in (unpublished)" \
      org.opencontainers.image.revision="${VSIFT_COMMIT}"
ENV VSIFT_COMMIT=${VSIFT_COMMIT}
USER 10001:10001
WORKDIR /trials
ENTRYPOINT ["/opt/vsift-trials/bin/trial-driver"]
CMD ["help"]

# ---------------------------------------------------------------------------
# `harness`: the agent image plus the repository at the commit under test,
# for `prepare` and `grade` only, which read the corpus, its truth, the
# scenarios and the skill. No client ever runs in it.
FROM agent AS harness
ARG VSIFT_COMMIT=unknown
COPY --chown=root:root . /opt/vsift/src
RUN /opt/vsift-trials/bin/vsift-agent-trials check-scenarios --repository /opt/vsift/src
LABEL org.opencontainers.image.title="vsift-codex-trials-harness" \
      org.opencontainers.image.description="P12 Codex trials: prepare and grade, never a client (unpublished)" \
      org.opencontainers.image.revision="${VSIFT_COMMIT}"

# ---------------------------------------------------------------------------
# P14. Node.js, for npm and for the launcher: the official linux-x64 tarball of
# the version the release workflow publishes with, by its published SHA-256.
FROM build-base AS node-fetch
ARG NODE_VERSION=24.21.0
ARG NODE_SHA256=fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6
RUN curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
      --max-filesize 80000000 --output /tmp/node.tar.xz \
      "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
 && echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum --check --strict \
 && mkdir -p /out/opt/node \
 && tar -xJf /tmp/node.tar.xz -C /out/opt/node --strip-components=1 --no-same-owner \
 && test -x /out/opt/node/bin/node \
 && test -f /out/opt/node/lib/node_modules/npm/bin/npm-cli.js \
 && rm /tmp/node.tar.xz

# ---------------------------------------------------------------------------
# P14. What both published images share: Ubuntu, the trial operator, Node.js,
# the harness, and VSift installed from the real registry. `install` runs npm
# with a cleared environment and an empty .npmrc, then reads the install back:
# the registry's integrity of each package against what npm installed, the
# launcher's digest check redone, and `vsift --version` through the launcher.
# It fails the build if the install is not the published package.
FROM ${UBUNTU_IMAGE} AS published-runtime
ARG VSIFT_PUBLISHED_VERSION
ENV DEBIAN_FRONTEND=noninteractive
COPY --from=build-base /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN sed -i 's#http://#https://#g' /etc/apt/sources.list.d/ubuntu.sources \
 && apt-get update \
 && apt-get install --yes --no-install-recommends ca-certificates curl libssl3t64 \
 && rm -rf /var/lib/apt/lists/*
RUN groupadd --gid 10001 trials \
 && useradd --uid 10001 --gid 10001 --home-dir /home/trials --create-home --shell /bin/bash trials \
 && install -d -o 10001 -g 10001 -m 0700 /trials /exports
COPY --from=node-fetch /out/ /
COPY --from=vsift-build /out/opt/vsift-trials /opt/vsift-trials
RUN test -n "${VSIFT_PUBLISHED_VERSION}" \
 && /opt/vsift-trials/bin/vsift-agent-trials install \
      --version "${VSIFT_PUBLISHED_VERSION}" \
      --prefix /opt/vsift-published \
      --node /opt/node/bin/node \
      --npm-cli /opt/node/lib/node_modules/npm/bin/npm-cli.js \
      --proof /opt/vsift-published-proof.json \
 && /opt/vsift-trials/bin/vsift-agent-trials verify-install --proof /opt/vsift-published-proof.json
ENV VSIFT_PUBLISHED_VERSION=${VSIFT_PUBLISHED_VERSION}

# ---------------------------------------------------------------------------
# P14 `agent-published`: the image Codex and its agent run in for the
# clean-install and cold-agent trials. VSift is the published package and
# nothing else of VSift: no repository, no corpus, no scenarios, no FFmpeg, no
# whisper.cpp, no model. The package ships the skill and a README; the cold
# agent must not be able to read them, so their folders and files are made
# unreadable to everyone but root (the bytes are unchanged: the launcher reads
# neither, and the proof above was taken before). The skill a skill trial uses
# is copied by `prepare` in the harness image.
FROM published-runtime AS agent-published
ARG VSIFT_COMMIT=unknown
COPY --from=codex-fetch /out/ /
COPY --chmod=0755 tools/vsift-agent-trials/containers/codex/trial-driver.sh /opt/vsift-trials/bin/trial-driver
RUN chmod -R go-rwx /opt/vsift-published/lib/node_modules/vsift-cli/skills \
 && find /opt/vsift-published/lib/node_modules -iname 'README*' -exec chmod go-rwx {} + \
 && /opt/node/bin/node /opt/vsift-published/lib/node_modules/vsift-cli/bin/vsift.cjs --version \
 && /opt/codex/bin/codex --version \
 && test ! -e /opt/vsift/src \
 && test ! -e /opt/ffmpeg \
 && test ! -e /opt/whisper.cpp
LABEL org.opencontainers.image.title="vsift-codex-trials-agent-published" \
      org.opencontainers.image.description="P14 Codex trials from the published package: the image the agent runs in (unpublished)" \
      org.opencontainers.image.revision="${VSIFT_COMMIT}"
ENV VSIFT_COMMIT=${VSIFT_COMMIT}
USER 10001:10001
WORKDIR /trials
ENTRYPOINT ["/opt/vsift-trials/bin/trial-driver"]
CMD ["help"]

# ---------------------------------------------------------------------------
# P14 `harness-published`: the published install plus the repository at the
# commit under test, for `prepare` and `grade` only. FFmpeg is here to build
# the looped and blurred clips; the tools a trial sees come from `setup
# install`, run by `prepare` as the user's part. No client ever runs in it.
FROM published-runtime AS harness-published
ARG VSIFT_COMMIT=unknown
COPY --from=ffmpeg-fetch /out/ /
COPY --chmod=0755 tools/vsift-agent-trials/containers/codex/trial-driver.sh /opt/vsift-trials/bin/trial-driver
COPY --chown=root:root . /opt/vsift/src
RUN /opt/vsift-trials/bin/vsift-agent-trials check-scenarios --repository /opt/vsift/src
LABEL org.opencontainers.image.title="vsift-codex-trials-harness-published" \
      org.opencontainers.image.description="P14 Codex trials from the published package: prepare and grade, never a client (unpublished)" \
      org.opencontainers.image.revision="${VSIFT_COMMIT}"
ENV VSIFT_COMMIT=${VSIFT_COMMIT}
USER 10001:10001
WORKDIR /trials
ENTRYPOINT ["/opt/vsift-trials/bin/trial-driver"]
CMD ["help"]
