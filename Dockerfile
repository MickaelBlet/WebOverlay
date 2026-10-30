# syntax=docker/dockerfile:1
# Desktop builds, one stage per OS so BuildKit runs them in parallel:
#   linux:    amd64 (deb, rpm)
#   appimage: amd64 (AppImage bundling glibc and every library, runs on any distro)
#   windows:  amd64 (NSIS installer + portable exe, cargo-xwin)
#   check:    fmt, clippy (Linux + Windows), tests; no output
# Usage: scripts/build.sh [--fast] [linux|windows]... (or `docker buildx bake [linux|appimage|windows]`),
#        scripts/check.sh (or `docker buildx bake check`).
# Output: dist-tauri/{linux,windows}/
# Base images and downloaded tools are pinned by digest or checksum: bump them on purpose.

ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
# Pinned: a new rustc invalidates the whole target caches.
ARG RUST_VERSION=1.99.1
ARG TAURI_CLI_VERSION=2.12.4
ARG TAURI_CLI_SHA256=820ae3c7580ca637cad15f65638960ee2a96e5a491e224691c86d6f25e6c9e8a

# Ubuntu 22.04: oldest base with webkit2gtk-4.1, keeps the deb/rpm glibc requirement low (2.35).
FROM ubuntu:22.04@sha256:5ec03bb3441e8b0bf3b4f9cd4629a1ae763010dc3035bb8da3ae6cf026486401 AS toolchain
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl file wget pkg-config xdg-utils \
      libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
      clang lld llvm nsis \
      fonts-noto-color-emoji \
    && rm -rf /var/lib/apt/lists/*

# Jammy ships NSIS 3.08, Tauri's installer script includes Win/RestartManager.nsh (NSIS >= 3.09).
ARG NSIS_DEB=nsis-common_3.09-4ubuntu1_all.deb
ARG NSIS_SHA256=49471b9debdee4751ebb4caf6b705cc4c678eebfaecefbe56362258893f85859
RUN cd /tmp && curl -fsSLO https://archive.ubuntu.com/ubuntu/pool/universe/n/nsis/${NSIS_DEB} \
    && echo "${NSIS_SHA256}  ${NSIS_DEB}" | sha256sum -c - \
    && dpkg-deb -x ${NSIS_DEB} nsis \
    && cp nsis/usr/share/nsis/Include/Win/RestartManager.nsh /usr/share/nsis/Include/Win/ \
    && rm -rf /tmp/nsis*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
# llvm-14/bin: clang-cl, lld-link, llvm-lib, llvm-rc used by cargo-xwin.
ENV PATH=/usr/local/cargo/bin:/usr/lib/llvm-14/bin:$PATH
ARG RUSTUP_VERSION RUSTUP_SHA256 RUST_VERSION
RUN curl -fsSLo /tmp/rustup-init \
      https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init \
    && echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain ${RUST_VERSION} \
      --target x86_64-unknown-linux-gnu --target x86_64-pc-windows-msvc \
      --component clippy --component rustfmt \
    && rm /tmp/rustup-init

# Prebuilt release binaries instead of `cargo install` (minutes of compilation each).
ARG TAURI_CLI_VERSION TAURI_CLI_SHA256
ARG CARGO_XWIN_VERSION=0.23.1
ARG CARGO_XWIN_SHA256=c492c6dfb7e5ac0eee586b796e0fc950ba13077e7f0bbb046f445d71790d5360
RUN cd /tmp \
    && curl -fsSLo tauri.tgz \
      https://github.com/tauri-apps/tauri/releases/download/tauri-cli-v${TAURI_CLI_VERSION}/cargo-tauri-x86_64-unknown-linux-gnu.tgz \
    && echo "${TAURI_CLI_SHA256}  tauri.tgz" | sha256sum -c - \
    && tar -xzf tauri.tgz -C $CARGO_HOME/bin cargo-tauri \
    && curl -fsSLo xwin.tgz \
      https://github.com/rust-cross/cargo-xwin/releases/download/v${CARGO_XWIN_VERSION}/cargo-xwin-v${CARGO_XWIN_VERSION}.x86_64-unknown-linux-musl.tar.gz \
    && echo "${CARGO_XWIN_SHA256}  xwin.tgz" | sha256sum -c - \
    && tar -xzf xwin.tgz -C $CARGO_HOME/bin cargo-xwin \
    && rm tauri.tgz xwin.tgz \
    && cargo tauri --version && cargo xwin --version

ENV XWIN_ACCEPT_LICENSE=1

# Crates downloaded once, into a layer (rebuilt only when Cargo.toml/Cargo.lock change): each OS
# build gets its own copy-on-write CARGO_HOME, so the concurrent builds don't race on unpack.
FROM toolchain AS deps
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && touch src/main.rs && cargo fetch --locked && rm -rf src

FROM deps AS src
COPY build.rs tauri.conf.json ./
COPY src src
COPY dist dist
COPY icons icons

# Separate target and tool caches per OS (the two builds run concurrently).
# FAST=1: thin LTO + 16 codegen units (much faster final codegen, slightly bigger binary), with its
# own target cache so switching modes doesn't rebuild every dependency.
FROM src AS linux
ARG FAST=
RUN --mount=type=cache,id=weboverlay-cache-linux,target=/root/.cache \
    --mount=type=cache,id=weboverlay-target-linux${FAST:+-fast},target=/app/target \
    set -eux; \
    [ -z "$FAST" ] || export CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16; \
    cargo tauri build --bundles deb,rpm -- --locked; \
    b=target/release/bundle; \
    mkdir -p /out; \
    cp $b/deb/*.deb $b/rpm/*.rpm /out/

FROM src AS windows
ARG FAST=
RUN --mount=type=cache,id=weboverlay-cache-windows,target=/root/.cache \
    --mount=type=cache,id=weboverlay-target-windows${FAST:+-fast},target=/app/target \
    set -eux; \
    [ -z "$FAST" ] || export CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16; \
    cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis -- --locked; \
    t=target/x86_64-pc-windows-msvc/release; \
    mkdir -p /out; \
    cp $t/bundle/nsis/*.exe /out/; \
    cp $t/weboverlay.exe /out/WebOverlay-portable.exe

# Portable AppImage: Ubuntu 22.04 libraries would need glibc >= 2.35 on the host, so it is built
# with sharun (quick-sharun), which also bundles glibc and its loader: runs on any distro, whatever
# its glibc (or musl). quick-sharun expects an Arch Linux build host; packages come from an Arch Linux
# Archive snapshot so the build is reproducible (bump ARCH_SNAPSHOT with the image digest).
FROM archlinux:base-devel@sha256:624bdc7fc6615155d67bc34cf7146c0c9fd0e3c92943977da63a630e4ca49c30 AS appimage-toolchain
ARG ARCH_SNAPSHOT=2026/10/27
RUN echo "Server = https://archive.archlinux.org/repos/${ARCH_SNAPSHOT}/\$repo/os/\$arch" \
      > /etc/pacman.d/mirrorlist \
    && pacman -Syyuu --noconfirm --needed \
      webkit2gtk-4.1 gtk3 librsvg xdotool openssl \
      patchelf strace xorg-server-xvfb dbus wget file \
    && pacman -Scc --noconfirm

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
ARG RUSTUP_VERSION RUSTUP_SHA256 RUST_VERSION TAURI_CLI_VERSION TAURI_CLI_SHA256
RUN curl -fsSLo /tmp/rustup-init \
      https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init \
    && echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain ${RUST_VERSION} \
    && rm /tmp/rustup-init \
    && curl -fsSLo /tmp/tauri.tgz \
      https://github.com/tauri-apps/tauri/releases/download/tauri-cli-v${TAURI_CLI_VERSION}/cargo-tauri-x86_64-unknown-linux-gnu.tgz \
    && echo "${TAURI_CLI_SHA256}  /tmp/tauri.tgz" | sha256sum -c - \
    && tar -xzf /tmp/tauri.tgz -C $CARGO_HOME/bin cargo-tauri \
    && rm /tmp/tauri.tgz

# Pinned commit; quick-sharun checks the sha256 of everything it downloads (sharun, appimagetool...).
ARG QUICK_SHARUN_COMMIT=1b27eda847d4ebaf046466544d94596c747380a5
ARG QUICK_SHARUN_SHA256=0aa3286d3329e0ead74ae8b3a5281f6210eda8cbc413e8d254c00316519111bb
RUN curl -fsSLo /usr/local/bin/quick-sharun \
      https://raw.githubusercontent.com/pkgforge-dev/Anylinux-AppImages/${QUICK_SHARUN_COMMIT}/useful-tools/quick-sharun.sh \
    && echo "${QUICK_SHARUN_SHA256}  /usr/local/bin/quick-sharun" | sha256sum -c - \
    && chmod +x /usr/local/bin/quick-sharun

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && touch src/main.rs && cargo fetch --locked && rm -rf src
COPY build.rs tauri.conf.json ./
COPY src src
COPY dist dist
COPY icons icons

FROM appimage-toolchain AS appimage
ARG FAST=
# quick-sharun runs the app for a few seconds (in Xvfb, launcher window) and records what it dlopens.
RUN --mount=type=cache,id=weboverlay-cache-appimage,target=/root/.cache \
    --mount=type=cache,id=weboverlay-target-appimage${FAST:+-fast},target=/app/target \
    set -eux; \
    [ -z "$FAST" ] || export CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16; \
    cargo tauri build --no-bundle -- --locked; \
    install -Dm755 target/release/weboverlay /usr/bin/weboverlay; \
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1); \
    mkdir -p /tmp/appimage; \
    cd /tmp/appimage; \
    printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Name=WebOverlay' \
      'Comment=Open any website as a standalone webapp with injected JS/CSS' \
      'Exec=weboverlay' 'Icon=weboverlay' 'Categories=Network;' 'Terminal=false' \
      'StartupWMClass=weboverlay' > weboverlay.desktop; \
    cp /app/icons/icon.png weboverlay.png; \
    export DESKTOP=$PWD/weboverlay.desktop ICON=$PWD/weboverlay.png VERSION=$version \
      OUTPATH=$PWD/dist OUTNAME=WebOverlay_${version}_amd64.AppImage; \
    quick-sharun /usr/bin/weboverlay; \
    quick-sharun --make-appimage; \
    mkdir -p /out; \
    cp dist/*.AppImage /out/

# Pre-commit checks; own caches so it can run alongside the release builds.
FROM src AS check
RUN --mount=type=cache,id=weboverlay-cache-check,target=/root/.cache \
    --mount=type=cache,id=weboverlay-target-check,target=/app/target \
    set -eux; \
    cargo fmt --check; \
    cargo clippy --locked --all-targets -- -D warnings; \
    cargo test --locked; \
    cargo xwin clippy --locked --target x86_64-pc-windows-msvc -- -D warnings

FROM scratch AS linux-out
COPY --from=linux /out /
COPY --from=appimage /out /

FROM scratch AS windows-out
COPY --from=windows /out /
