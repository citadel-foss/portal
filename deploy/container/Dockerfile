# Two build stages, one slim runtime. Only portal-web is built: the desktop package pulls in
# GTK and WebKit, none of which a headless server has any use for.

FROM node:22-bookworm-slim AS frontend
WORKDIR /build
COPY package.json package-lock.json ./
RUN npm ci
COPY tsconfig.json tsconfig.node.json vite.config.ts index.html ./
COPY src ./src
RUN npx tsc --noEmit && npx vite build --mode web

FROM rust:1-bookworm AS backend
WORKDIR /build
# Embedded Tor is built from source by libtor, so its toolchain has to be present here and
# nowhere in the runtime image.
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config libssl-dev build-essential && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY core ./core
COPY src-web ./src-web
COPY src-tauri ./src-tauri
COPY contracts ./contracts
# The desktop member stays in the workspace: removing it rewrites the dependency graph, and
# --locked then refuses the lockfile change. `-p portal-web` already builds only what the
# server needs, so src-tauri is present but never compiled.
RUN cargo build --locked --release -p portal-web

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates && rm -rf /var/lib/apt/lists/*
# Unprivileged, with a home the protocol crate resolves its own data root under. Set before
# the process starts rather than mutated afterwards: the crate reads it during init, and
# changing HOME once threads are running splits the tree.
RUN useradd --system --create-home --home-dir /data/home --uid 10001 portal
COPY --from=backend /build/target/release/portal-web /usr/local/bin/portal-web
COPY --from=frontend /build/dist/web /opt/portal/web
USER portal
ENV HOME=/data/home
VOLUME ["/data"]
EXPOSE 3000
# Probes the real endpoint: `--help` exits zero even when the server is wedged, which would
# report healthy forever. Public, cached and side-effect free by design — it never starts Tor
# or touches a wallet lock.
HEALTHCHECK --interval=30s --timeout=3s --start-period=20s \
  CMD ["/usr/local/bin/portal-web", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/portal-web"]
CMD ["--bind", "0.0.0.0:3000", "--assets-dir", "/opt/portal/web"]
