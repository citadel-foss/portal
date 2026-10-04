<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="Portal" width="110" />

# Portal

A Bitcoin wallet that swaps your coins privately, over Tor, with no trusted third party.
Runs as a desktop app, or as a server you host yourself and reach from a browser.

[![Latest release](https://img.shields.io/github/v/release/citadel-foss/portal?label=release)](https://github.com/citadel-foss/portal/releases/latest)
[![MIT Licensed](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Website](https://img.shields.io/badge/website-citadelfoss.xyz-blue)](https://citadelfoss.xyz/)

</div>

## ⚠️ Notice

The app is in active beta and supports experimental mainnet use.

<p align="center">
  <img src="docs/images/wallet.jpg" alt="Portal's wallet page" width="96%" />
</p>
<p align="center">
  <img src="docs/images/swap-complete.jpg" alt="A completed swap through three routers" width="31%" />
  <img src="docs/images/router-fleet.jpg" alt="The router fleet" width="64%" />
</p>

# About

Portal is a desktop and self-hosted wallet for [OpenSwap](https://github.com/citadel-foss/openswap),
a trustless, self-custodial [atomic swap](https://bitcoinops.org/en/topics/coinswap/) protocol
built on Bitcoin. Unlike swap services that rely on a central server as a
[single point of failure](https://en.wikipedia.org/wiki/Single_point_of_failure), OpenSwap's
marketplace lives in the Bitcoin blockchain itself: there is no central host, and anyone with a
Bitcoin node can take part.

It is a full wallet (receive, send, coin control, history) with swaps built in. You choose one of
two roles at launch:

- **Wallet** (a *taker* in the protocol) starts swaps. It pays the fees (swap and mining), needs
  no bond, and picks routers by bond validity, available liquidity and fee rates.
- **Router** (a *maker* in the protocol) supplies liquidity and earns a fee on every swap routed
  through it. Routers compete on fees in an open market and run in *install, fund, forget* mode,
  with their liquidity kept in the router's hot wallet.

**Multi-hop routing** works like Lightning: each swap passes through several routers, and no single
router sees the whole route. Your wallet relays every message between them over Tor. You can use
the modern Taproot + MuSig2 contracts or the legacy P2WSH ones.

**Sybil resistance** comes from
[fidelity bonds](https://github.com/JoinMarket-Org/joinmarket-clientserver/blob/master/docs/fidelity-bonds.md):
time-locked UTXOs that make flooding the market with fake routers expensive, and that seed the
marketplace on-chain.

# Download

Get Portal for macOS, Linux, a self-hosted server or Docker from the
[latest release](https://github.com/citadel-foss/portal/releases/latest), or install it from
the Umbrel App Store. Release builds bundle everything they need, Tor included.

New to Portal? The [user guide](docs/guide.md) walks you from first launch to your first swap.

# Build from source

## Requirements

- **Node.js** 20.19 or newer
- **Rust** via [rustup](https://rustup.rs/). The version is pinned in `rust-toolchain.toml` and
  picked up automatically.
- **System dependencies:**

```bash
# macOS
xcode-select --install

# Debian / Ubuntu
sudo apt-get update
sudo apt-get install -y build-essential curl wget file libssl-dev libayatana-appindicator3-dev \
  librsvg2-dev libwebkit2gtk-4.1-dev libxdo-dev pkg-config
```

Tauri's [prerequisites guide](https://tauri.app/start/prerequisites/) covers other platforms.

## Desktop app

```bash
git clone https://github.com/citadel-foss/portal.git
cd portal
npm install
npm run tauri dev
```

The first run compiles the Rust backend and the OpenSwap library, which takes a few minutes.
Later runs are incremental.

`npm run tauri build` produces installers for your platform in `target/release/bundle/`.

## Server version

```bash
npm install
npm run web:dev
```

Then open <http://localhost:1430>.

`npm run web:build` produces the release server: a single binary at
`target/release/portal-web` with the frontend built in. Run it with `--help` for its options.

To build and run the Docker image locally:

```bash
docker compose -f umbrel/compose.local.yaml up --build
```

Then open <http://localhost:3000>.

## Layout

```
src/        React frontend (src/api/ is the typed boundary to the backend)
core/       wallet, swaps and Tor, shared by both hosts
src-tauri/  desktop host
src-web/    server host
umbrel/     container image and Umbrel packaging
```

Each wallet and router keeps its data in `~/.openswap/`, in the same layout the OpenSwap
command-line tools use.

# Links

- [User guide](docs/guide.md)
- [OpenSwap](https://github.com/citadel-foss/openswap) and its
  [protocol specification](https://github.com/citadel-foss/OpenSwap-Protocol-Specification)
- [Website](https://openswap.live/portal) · [Matrix](https://matrix.to/#/#ciatdel-foss:matrix.org)
