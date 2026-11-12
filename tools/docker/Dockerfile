# SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# The base image by digest (the multi-platform index of ubuntu:24.04 on 2026-11-12; the build
# selects linux/amd64 from it).
FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5

ARG TOOLCHAIN_SHA256
ARG TOOLCHAIN_URL=https://pkgman.jieliapp.com/s/linux-toolchain
ARG RUST_VERSION=1.99.0

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
    build-essential cmake ninja-build clang libclang-dev pkg-config git jq curl \
    xz-utils unzip ca-certificates python3 \
 && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION} \
        --target wasm32-unknown-unknown \
 && chmod -R a+rwX /usr/local/rustup /usr/local/cargo

RUN test -n "${TOOLCHAIN_SHA256}" \
 && curl --proto '=https' --tlsv1.2 -fsSL -o /tmp/jieli.tar.xz "${TOOLCHAIN_URL}" \
 && echo "${TOOLCHAIN_SHA256}  /tmp/jieli.tar.xz" | sha256sum -c - \
 && mkdir -p /opt/jieli \
 && tar -xJf /tmp/jieli.tar.xz -C /opt/jieli \
 && ln -s /opt/jieli/jieli-linux-toolchains-* /opt/jieli/toolchain \
 && rm /tmp/jieli.tar.xz

ENV JIELI_TOOLCHAIN=/opt/jieli/toolchain FMZX_BUILD_IMAGE=1
RUN git config --system --add safe.directory '*'
