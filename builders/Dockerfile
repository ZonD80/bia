# Unified bia builder: Debian (glibc) for all Rust bins + debootstrap guest rootfs.
# Agent/MCPs are dynamically linked; telegram uses libntgcalls.so from speech tree.
# Bootloader (shim + signed GRUB + BIOS) is installed into the guest rootfs from Debian.
FROM debian:bookworm-slim
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential ca-certificates curl git pkg-config \
    mold unzip xz-utils zstd rsync findutils python3 \
    e2fsprogs dosfstools \
    libsqlite3-dev libopus-dev \
    debootstrap debian-archive-keyring dpkg \
 && rm -rf /var/lib/apt/lists/* \
 && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain 1.94.0 --profile minimal \
 && . /root/.cargo/env \
 && rustc --version && cargo --version \
 && mold --version
ENV CARGO_HOME=/cache/cargo
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /work
