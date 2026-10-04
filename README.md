<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="Portal" width="110" />

# Portal

One dashboard for running OpenSwap wallets and routers.
Make atomic swaps, explore the market, manage multiple routers, and earn sats by providing liquidity.
Ships as a native desktop app, or as a web app for headless servers.

[![Latest pre-release](https://img.shields.io/github/v/release/citadel-foss/portal?include_prereleases&label=pre-release)](https://github.com/citadel-foss/portal/releases)
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

Portal is a unified app for [OpenSwap](https://github.com/citadel-foss/openswap), a trustless,
self-custodial [atomic swap](https://bitcoinops.org/en/topics/coinswap/) protocol built on Bitcoin.
It brings everything OpenSwap into a single interface.

OpenSwap is a decentralised atomic swap market built on Bitcoin and Lightning, where anyone can take
part as a client or a server. Unlike swap services that rely on a central server as a
[single point of failure](https://en.wikipedia.org/wiki/Single_point_of_failure), OpenSwap has no
central host: the market lives on Nostr and can be recovered from Bitcoin blockchain data alone.

Portal is a full-featured Bitcoin wallet with built-in atomic swaps and market discovery. It runs in
two roles:

- **Wallet** (a *client* in the protocol): performs swaps, pays swap fees, discovers and selects
  makers from the market, and manages UTXOs.
- **Router** (a *server* in the protocol): supplies liquidity, advertises fidelity bonds, and earns
  fees from swaps. A router runs as a self-hosted, always-on node with its liquidity in a hot wallet.

A single Portal app manages multiple wallets and routers. It builds as a native desktop app, or as a
web app for headless servers.

Portal connects to third-party or self-hosted Electrum servers, or directly to a Bitcoin full node.
All network traffic goes over Tor, and Portal manages Tor itself, so no local setup is needed.

Portal is built entirely on the OpenSwap Core APIs and the Rust-based Tauri framework. The result is
a small, fast, cross-platform binary that runs comfortably on a low-cost VPS or a Raspberry Pi at
home: plug-and-play, with no heavy setup or ongoing management.

New to Portal? The [user guide](docs/guide.md) walks you from first launch to your first swap.

# Download

Get Portal Desktop or Portal Server for macOS or Linux from the
[latest pre-release](https://github.com/citadel-foss/portal/releases).

# Build from source

## Prerequisites

### Toolchain

- **Node.js** 20.19 or newer
- **Rust** via [rustup](https://rustup.rs/). The version is pinned in `rust-toolchain.toml` and
  picked up automatically.

### System dependencies

**macOS**

```bash
xcode-select --install
```

**Debian / Ubuntu**

```bash
sudo apt-get update
sudo apt-get install -y build-essential curl wget file libssl-dev libayatana-appindicator3-dev \
  librsvg2-dev libwebkit2gtk-4.1-dev libxdo-dev pkg-config
```

For other platforms, see Tauri's [prerequisites guide](https://tauri.app/start/prerequisites/).

## Get the source

```bash
git clone https://github.com/citadel-foss/portal.git
cd portal
npm install
```

The first build compiles the Rust backend and the OpenSwap library, which takes a few minutes.
Later builds are incremental.

## Portal Desktop

### Run in development

```bash
npm run tauri dev
```

### Build installers

```bash
npm run tauri build
```

Installers for your platform are written to `target/release/bundle/`.

## Portal Server

### Run in development

```bash
npm run web:dev
```

Then open <http://localhost:1430>.

### Build the release binary

```bash
npm run web:build
```

This produces a single binary at `target/release/portal-web` with the frontend built in. To see
its options:

```bash
./target/release/portal-web --help
```

### Run with Docker

Build and run the Docker image locally:

```bash
docker compose -f umbrel/compose.local.yaml up --build
```

Then open <http://localhost:3000>.

## Repository layout

```text
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
