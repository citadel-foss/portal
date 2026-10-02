#!/usr/bin/env python3
"""Collect every supported platform before replacing the public prerelease."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tarfile
from datetime import datetime, timezone


INSTALLERS = (
    ("portal-linux-*-appimage", "*.AppImage", "portal-linux-x86_64.AppImage", "Linux x86_64", "Desktop"),
    ("portal-linux-*-deb", "*.deb", "portal-linux-x86_64.deb", "Linux x86_64", "Desktop"),
    ("portal-linux-*-rpm", "*.rpm", "portal-linux-x86_64.rpm", "Linux x86_64", "Desktop"),
    ("portal-mac-universal-dmg", "*.dmg", "portal-macos-universal.dmg", "macOS Universal", "Desktop"),
    ("portal-mac-universal-app-archive", "*.app.tar.gz", "portal-macos-universal.app.tar.gz", "macOS Universal", "Desktop"),
)
SERVERS = (
    ("linux-x86_64", "Linux x86_64"),
    ("linux-arm64", "Linux arm64"),
    ("mac-universal", "macOS Universal"),
)


def exactly_one(paths, description):
    paths = list(paths)
    if len(paths) != 1 or not paths[0].is_file():
        raise RuntimeError(f"Expected exactly one {description}; found {len(paths)}")
    return paths[0]


def package(downloads, output):
    # Validate the complete set before creating any release assets.
    installers = [
        (exactly_one(downloads.glob(f"{artifact}/**/{pattern}"), name), name, platform, kind)
        for artifact, pattern, name, platform, kind in INSTALLERS
    ]
    servers = [
        (exactly_one(downloads.glob(f"portal-server-{asset}/**/portal-server"), asset), asset, platform)
        for asset, platform in SERVERS
    ]
    output.mkdir(parents=True, exist_ok=False)
    assets = []
    for source, name, platform, kind in installers:
        shutil.copyfile(source, output / name)
        assets.append((name, platform, kind))
    for source, asset, platform in servers:
        name = f"portal-server-{asset.replace('mac-', 'macos-')}.tar.gz"
        with tarfile.open(output / name, "w:gz") as archive:
            info = archive.gettarinfo(str(source), arcname="portal-server")
            # The artifact store removes executable bits; the archive restores them.
            info.mode = 0o755
            with source.open("rb") as binary:
                archive.addfile(info, binary)
        assets.append((name, platform, "Server"))
    return assets


def describe(output, assets, repository, tag, openswap_sha, portal_sha, run_url, version):
    base = f"https://github.com/{repository}/releases/download/{tag}"
    entries = []
    for name, platform, kind in assets:
        checksum = hashlib.sha256((output / name).read_bytes()).hexdigest()
        entries.append({"name": name, "platform": platform, "kind": kind,
                        "sha256": checksum, "url": f"{base}/{name}"})
    manifest = {
        "openswap_sha": openswap_sha, "portal_sha": portal_sha, "portal_version": version,
        "built_at": datetime.now(timezone.utc).isoformat(), "workflow_run": run_url,
        "assets": entries,
    }
    (output / "build.json").write_text(json.dumps(manifest, indent=2) + "\n")
    files = sorted(output.iterdir())
    (output / "SHA256SUMS").write_text("".join(
        f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in files
    ))
    notes = [
        "Latest successful Portal build against OpenSwap master. This is a prerelease.", "",
        f"- OpenSwap: [{openswap_sha}](https://github.com/citadel-foss/openswap/commit/{openswap_sha})",
        f"- Portal: [{portal_sha}](https://github.com/{repository}/commit/{portal_sha})",
        f"- App version: `{version}`",
        f"- [Build and logs]({run_url})", "",
        "| Platform | Package | Download |", "| --- | --- | --- |",
    ]
    notes.extend(f"| {entry['platform']} | {entry['kind']} | [{entry['name']}]({entry['url']}) |"
                 for entry in entries)
    notes.extend(["", f"[SHA256SUMS]({base}/SHA256SUMS) · [Download manifest]({base}/build.json)", "",
                  "Server archives preserve executable permissions. macOS packages use the same ad-hoc signing as Portal's existing builds.", ""])
    return "\n".join(notes)


if __name__ == "__main__":
    downloads, output = map(Path, sys.argv[1:])
    assets = package(downloads, output)
    repository = os.environ["GITHUB_REPOSITORY"]
    run_url = f"https://github.com/{repository}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    version = json.loads(Path("src-tauri/tauri.conf.json").read_text())["version"]
    notes = describe(output, assets, repository, os.environ["RELEASE_TAG"],
                     os.environ["OPENSWAP_SHA"], os.environ["PORTAL_SHA"], run_url, version)
    Path("release-notes.md").write_text(notes)
    print(notes)
