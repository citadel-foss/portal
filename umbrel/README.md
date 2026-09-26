# Portal for Umbrel

This folder replaces `deploy/container`. It builds **Portal web**, including its
frontend, Rust server and embedded Tor. The desktop app is not built.

```text
umbrel/
  Dockerfile                 Build recipe; uses the Portal repository as context
  compose.local.yaml         Standalone localhost smoke test
  compose.standalone.yaml    Standalone external TLS proxy deployment
  STANDALONE.md              Standalone usage and data notes
  portal/                   Copy this directory into an Umbrel app-store source
    docker-compose.yml      Image, Umbrel proxy, environment and persistent mount
    umbrel-app.yml           App metadata and browser launch port
    data/portal/.gitkeep     Tracks the otherwise-empty persistent data directory
```

The Dockerfile is a recipe. Build it to produce an **image**, then push that image
to a registry for public distribution. Umbrel pulls the image; it does not build
the Dockerfile. The `portal/` package is portable once its image is available.
Moving the build recipe alone does not copy the application source: always use
the Portal source repository as the Docker build context.

## Local image for the existing OrbStack test instance

Run from the Portal repository root:

```sh
docker build -f umbrel/Dockerfile -t portal:umbrel-test .
docker save portal:umbrel-test | docker exec -i portal-umbrel-test docker load
```

The two Docker engines have separate image stores. Loading into Umbrel's engine
is required even though the outer OrbStack engine already has the image.

After completing Umbrel onboarding, copy `portal/` into the recognized app-store
source in the disposable test instance. For the current official store checkout:

```sh
docker cp umbrel/portal portal-umbrel-test:/home/umbrel/umbrel/app-stores/getumbrel-umbrel-apps-github-53f74447/portal
docker exec portal-umbrel-test umbreld client appStore.registry.query
docker exec portal-umbrel-test umbreld client apps.install.mutate --appId portal
```

Confirm Portal appears in the registry before installing. The copy command is for
the first copy into a missing `portal` directory; for subsequent changes sync its
contents rather than creating a nested `portal/portal`. A store refresh may replace
local edits; this is a disposable local test workflow, not a publication method.

Open Portal from the Umbrel dashboard, normally at `http://umbrel.local:3100`.
Use the configured device hostname, not localhost or an IP alias: Portal checks
the browser origin. This package targets the HTTP LAN route behind Umbrel auth;
HTTPS and onion browser access require separate origin/profile configuration and
verification. Portal's embedded Tor for swaps is independent of browser access.

Create the Portal owner password, connect the chain backend, and verify Tor,
wallet setup and persistence through an Umbrel restart. Umbrel login protects the
route; Portal's owner password and wallet passwords remain separate.

`app_proxy` intentionally has no image in the source Compose file: Umbrel injects
its proxy implementation. Do not run this package directly with Docker Compose;
use `compose.local.yaml` for a standalone smoke test.

## Public registry and App Store submission

The checked-in package currently uses `portal:umbrel-test`. It is **not yet a
public release package**. Before submission:

1. Build and publish both architectures to a registry namespace you control:

   ```sh
   docker buildx build --platform linux/amd64,linux/arm64 \
     -f umbrel/Dockerfile -t ghcr.io/YOUR_OWNER/portal:0.1.0 --push .
   docker buildx imagetools inspect ghcr.io/YOUR_OWNER/portal:0.1.0
   ```

2. Make the registry package publicly readable. Replace the `image` in
   `portal/docker-compose.yml` with `ghcr.io/YOUR_OWNER/portal:0.1.0@sha256:...`,
   using the actual multi-architecture index digest printed by inspect. Do not
   use an architecture-specific digest. Match the manifest version to the release.
3. Confirm the external port is free in the current App Store, confirm submitter
   metadata, and fill `submission` with the real PR URL. Supply screenshots and a
   logo to the reviewers; Umbrel manages official gallery assets.
4. Copy `portal/` into a checkout of `getumbrel/umbrel-apps`. Run its package
   linter: `npm run lint:apps -- portal --check-images`. Test a fresh installation
   through Umbrel with the published image, including onboarding, connectivity,
   restart and persistent data. Record which architectures were actually tested.
5. Submit that `portal/` directory to `getumbrel/umbrel-apps`.

No registry upload or app installation is performed by merely copying this folder.

## Persistent data

`${APP_DATA_DIR}/data/portal` is mounted at `/data`. It holds Portal's home directory
and `.openswap` state. The image uses UID/GID 1000:1000 to match Umbrel's package
directory ownership. `.gitkeep` is empty and executes nothing; Umbrel removes it
when installing. Keep all wallet/recovery state in the persistent mount. Take
service backups while Portal is stopped; wallet exports alone are not full backups.

## References

- [Umbrel packaging requirements](https://github.com/getumbrel/umbrel-apps/blob/master/.claude/skills/umbrel-package-app/SKILL.md)
- [Umbrel testing workflow](https://github.com/getumbrel/umbrel-apps/blob/master/.claude/skills/umbrel-test-app/SKILL.md)
