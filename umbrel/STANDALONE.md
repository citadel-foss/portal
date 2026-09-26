# Standalone container

Portal speaks plain HTTP and expects a TLS-terminating reverse proxy in front of it. The
container publishes no port of its own; only the proxy should be able to reach it.

## Trying it locally

One command builds the image and runs it; the frontend, the Rust binary and the runtime are
all stages of the one Dockerfile, so there is nothing to build beforehand.

```sh
docker compose -f umbrel/compose.local.yaml up --build
```

Then open <http://localhost:3000>. Ctrl+C runs the same ordered teardown a supervisor would
trigger; the log is inside the volume at `/data/home/.openswap/taker/debug.log`.

This uses the `trusted-http-proxy` profile, which is the shape an app-store wrapper installs
under: a reachable bind, a plain-HTTP origin, and no `Secure` on the session cookie. Keep it
bound to `127.0.0.1` as the compose file does — the cookie is unencrypted, so anything wider
than one host wants `trusted-tls-proxy` and a real proxy instead.

## First run

```sh
PORTAL_PUBLIC_ORIGIN=https://portal.example docker compose -f umbrel/compose.standalone.yaml up -d
```

Open the origin and choose the owner password; every later visit signs in with it. Until that
first visit, whoever reaches the page first sets it, so open it yourself straight after the
first start. Forgot it? Stop the container, delete `/data/home/.openswap/taker/portal/auth/owner`
and start it again — wallets are untouched, each still behind its own wallet password.

## What is where

`/data` is the only volume worth backing up. It holds `home/.openswap/` — wallets, swap
tracker, reports, router registrations and the login verifier.

An encrypted wallet export is **not** a service backup: it has no tracker, recovery or router
state. Take a service backup with the container stopped.

## Not covered here

The Umbrel package is in `portal/`; see [README.md](README.md). StartOS packaging is not included.
