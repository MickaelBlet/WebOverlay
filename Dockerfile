# syntax=docker/dockerfile:1
# Desktop builds, one stage per OS so BuildKit runs them in parallel:
#   linux:   amd64 (deb, rpm; AppImage repackaged in the `appimage` stage)
#   windows: amd64 (NSIS installer + portable exe, cargo-xwin)
#   check:   fmt, clippy (Linux + Windows), tests; no output
# Usage: scripts/build.sh [--fast] [linux|windows]... (or `docker buildx bake [linux|windows]`),
#        scripts/check.sh (or `docker buildx bake check`).
# Output: dist-tauri/{linux,windows}/
# Base images and downloaded tools are pinned by digest or checksum: bump them on purpose.

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
ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
# Pinned: a new rustc invalidates the whole target caches.
ARG RUST_VERSION=1.99.1
RUN curl -fsSLo /tmp/rustup-init \
      https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init \
    && echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain ${RUST_VERSION} \
      --target x86_64-unknown-linux-gnu --target x86_64-pc-windows-msvc \
      --component clippy --component rustfmt \
    && rm /tmp/rustup-init

# Prebuilt release binaries instead of `cargo install` (minutes of compilation each).
ARG TAURI_CLI_VERSION=2.12.4
ARG TAURI_CLI_SHA256=820ae3c7580ca637cad15f65638960ee2a96e5a491e224691c86d6f25e6c9e8a
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
    b=target/release/bundle; \
    rm -rf $b; \
    cargo tauri build --bundles deb,rpm -- --locked; \
    mkdir -p /out /appimage-src; \
    cp $b/deb/*.deb $b/rpm/*.rpm /out/; \
    dpkg-deb -x $b/deb/*.deb /appimage-src; \
    dpkg-deb -f $b/deb/*.deb Version > /appimage-src/version

# Self-contained AppImage: the binary plus Arch Linux's webkit2gtk-4.1, GTK, Mesa and glibc itself,
# deployed by quick-sharun (sharun runs every bundled executable, WebKit's helper processes
# included, through the bundled dynamic loader), so it runs on old distros without webkit2gtk-4.1
# (RHEL/Rocky 8, glibc 2.28). Arch is quick-sharun's supported build host; the debloated
# packages (`continuous` release) only exist for current Arch, so this stage tracks rolling Arch.
FROM archlinux:latest@sha256:e82623936fa1a69700b48e98b058df6eaab18b737c27473a7234a40fdecb9413 AS appimage
ARG ANYLINUX_COMMIT=bec22efd769b122f1fa9c89606a2d969498a5930
ARG QUICK_SHARUN_SHA256=fa9dbe2532d568e1c1b8eeaba94d1c57109b776b5541d21b74d3f44b74faf4d3
ARG DEBLOATED_PKGS_SHA256=c8880a7e2f0a47b68ed5ae34e1b7aeecc2e5b3a55f3583b0b43d9818a15dbe8b
RUN pacman -Syu --noconfirm --needed \
      webkit2gtk-4.1 gtk3 binutils file patchelf wget xorg-server-xvfb xorg-xauth \
    && cd /usr/local/bin \
    && u=https://raw.githubusercontent.com/pkgforge-dev/Anylinux-AppImages/${ANYLINUX_COMMIT}/useful-tools \
    && curl -fsSLo quick-sharun $u/quick-sharun.sh \
    && curl -fsSLo get-debloated-pkgs $u/get-debloated-pkgs.sh \
    && printf '%s  %s\n' ${QUICK_SHARUN_SHA256} quick-sharun ${DEBLOATED_PKGS_SHA256} get-debloated-pkgs \
      | sha256sum -c - \
    && chmod +x quick-sharun get-debloated-pkgs \
    && get-debloated-pkgs --add-common --prefer-nano webkit2gtk-4.1-mini \
    && pacman -Scc --noconfirm
COPY --from=linux /appimage-src /appimage-src
WORKDIR /appimage
# STRACE_MODE (default) runs the app under Xvfb for a few seconds to collect dlopened libraries.
RUN set -eux; \
    cp -r /appimage-src/usr /; \
    export VERSION=$(cat /appimage-src/version) OUTPUT_APPIMAGE=1 OUTPATH=/out \
      DESKTOP=/usr/share/applications/WebOverlay.desktop \
      ICON=/usr/share/icons/hicolor/512x512/apps/weboverlay.png; \
    OUTNAME=WebOverlay_${VERSION}_amd64.AppImage quick-sharun /usr/bin/weboverlay; \
    find /out -type f ! -name "*.AppImage" -delete

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
