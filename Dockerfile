# syntax=docker/dockerfile:1
# Desktop builds, one stage per OS so BuildKit runs them in parallel:
#   linux:   amd64 (deb, rpm, AppImage)
#   windows: amd64 (NSIS installer + portable exe, cargo-xwin)
# Usage: scripts/build.sh (or `docker buildx bake`). Output: dist-tauri/{linux,windows}/
# Base images and downloaded tools are pinned by digest or checksum: bump them on purpose.

# Ubuntu 22.04: oldest base with webkit2gtk-4.1, keeps the AppImage glibc requirement low.
FROM ubuntu:22.04@sha256:5ec03bb3441e8b0bf3b4f9cd4629a1ae763010dc3035bb8da3ae6cf026486401 AS toolchain
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl file wget pkg-config xdg-utils \
      libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
      clang lld llvm nsis \
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
ARG TAURI_CLI_VERSION=2.12.4
RUN curl -fsSLo /tmp/rustup-init \
      https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init \
    && echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain stable \
      --target x86_64-unknown-linux-gnu --target x86_64-pc-windows-msvc \
    && rm /tmp/rustup-init \
    && cargo install --locked cargo-xwin \
    && cargo install --locked tauri-cli --version ${TAURI_CLI_VERSION} \
    && rm -rf $CARGO_HOME/registry

# linuxdeploy (AppImage) is itself an AppImage: no FUSE inside containers.
ENV APPIMAGE_EXTRACT_AND_RUN=1 XWIN_ACCEPT_LICENSE=1

FROM toolchain AS src
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs tauri.conf.json ./
COPY src src
COPY dist dist
COPY icons icons

# Separate registry, target and tool caches per OS: the two builds run concurrently, and cargo's
# package lock lives in CARGO_HOME outside the registry mount, so a shared registry races on unpack.
FROM src AS linux
RUN --mount=type=cache,id=weboverlay-registry-linux,target=/usr/local/cargo/registry \
    --mount=type=cache,id=weboverlay-cache-linux,target=/root/.cache \
    --mount=type=cache,id=weboverlay-target-linux,target=/app/target \
    set -eux; \
    cargo tauri build --bundles deb,rpm,appimage; \
    b=target/release/bundle; \
    mkdir -p /out/linux; \
    cp $b/deb/*.deb $b/rpm/*.rpm $b/appimage/*.AppImage /out/linux/

FROM src AS windows
RUN --mount=type=cache,id=weboverlay-registry-windows,target=/usr/local/cargo/registry \
    --mount=type=cache,id=weboverlay-cache-windows,target=/root/.cache \
    --mount=type=cache,id=weboverlay-target-windows,target=/app/target \
    set -eux; \
    cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis; \
    t=target/x86_64-pc-windows-msvc/release; \
    mkdir -p /out/windows; \
    cp $t/bundle/nsis/*.exe /out/windows/; \
    cp $t/weboverlay.exe /out/windows/WebOverlay-portable.exe

FROM scratch AS tauri
COPY --from=linux /out /
COPY --from=windows /out /
