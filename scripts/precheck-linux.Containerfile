# The Linux leg of `make precheck-full`: GitHub's ubuntu-24.04 runner, as far
# as the check job uses it, built by Apple's `container` CLI on the Mac.
#
# The packages are the ones ci.yml installs, plus what the hosted image has
# baked in that the build needs (cmake for libopus, git, python3, perl).
# The tests run as an unprivileged user, as on the runner: as root, a test
# that expects a permission to be refused would pass where CI fails.
FROM ubuntu:24.04

RUN apt-get update && apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
      patchelf build-essential libssl-dev libdbus-1-dev \
      ca-certificates curl git cmake pkg-config python3 perl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 1001 runner

# rustup in the runner's home, where GitHub's image has it: the workspace
# shell lets a sandboxed command run the toolchains it finds under home, and
# a test checks it does. CARGO_HOME's registry is a volume, which keeps
# downloaded crates warm across runs; the image keeps rustc.
ARG RUST_VERSION
USER runner
ENV PATH=/home/runner/.cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --profile minimal \
          --default-toolchain "$RUST_VERSION" --component clippy,rustfmt \
    && mkdir -p /home/runner/.cargo/registry
USER root
