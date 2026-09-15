# Multi-stage build: compile the Rust backend and embed the React frontend.
#
# Stage 1 — Node: build the Vite/React frontend (the compiled dist/ is baked
#            into the Rust binary at compile time via include_dir!).
# Stage 2 — Rust: compile the Rust backend (statically links OpenSSL so the
#            final image has no system-library dependencies beyond glibc).
# Stage 3 — Runtime: a slim Debian image with only what the process needs.

# ── Stage 1: frontend ────────────────────────────────────────────────────────
FROM node:20-slim AS frontend

WORKDIR /app

COPY package*.json ./
RUN npm ci --no-fund --no-audit

COPY src ./src
COPY public ./public
COPY index.html vite.config.ts tsconfig.json tsconfig.node.json ./

RUN npm run build

# ── Stage 2: backend ─────────────────────────────────────────────────────────
FROM rust:1-bookworm AS backend

# System libraries needed to compile the crate tree:
#   pkg-config     — used by several *-sys crates to locate libs
#   libssl-dev     — openssl-sys (even though we vendor OpenSSL, the build
#                    script still needs pkg-config to detect it cleanly)
#   libasound2-dev — cpal (ALSA audio; needed even if we're not using it
#                    because the crate is always compiled)
#   libmp3lame-dev — mp3lame-encoder
#   nasm / perl    — OpenSSL vendored build
RUN apt-get update -qq && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev libasound2-dev libmp3lame-dev nasm perl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the pre-built frontend so include_dir! can embed it.
COPY --from=frontend /app/dist ./dist

# Copy the Rust workspace.
COPY src-tauri ./src-tauri

# Vendor OpenSSL into the binary (no runtime .so dependency).
ENV OPENSSL_STATIC=1 OPENSSL_VENDOR=1

# Build in release mode.
RUN cd src-tauri && cargo build --release

# ── Stage 3: runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# ALSA libs for audio metering (loaded at runtime; absent = audio disabled,
# everything else still works). mp3lame is statically linked in the binary.
RUN apt-get update -qq && apt-get install -y --no-install-recommends \
    libasound2 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create an unprivileged user.
RUN useradd -m -u 1001 prodeck

# Copy the compiled binary.
COPY --from=backend /app/src-tauri/target/release/prodeck /usr/local/bin/prodeck

# Data directory — settings, PCO data, identity, etc. Mount a volume here.
RUN mkdir -p /home/prodeck/.config/ProDeck && chown -R prodeck:prodeck /home/prodeck

USER prodeck

# Default port; override with PRODECK_PORT env var or -e PRODECK_PORT=xxxx.
EXPOSE 4000

# Store ProDeck data at /data (mount a named volume or host path here).
VOLUME ["/home/prodeck/.config/ProDeck"]

ENV HOME=/home/prodeck

ENTRYPOINT ["/usr/local/bin/prodeck"]
