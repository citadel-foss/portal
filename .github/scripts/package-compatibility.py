#!/usr/bin/env python3
"""Collect every supported platform before replacing the public prerelease."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import urllib.error
import urllib.request
from datetime import datetime, timezone


INSTALLERS = (
    ("portal-linux-*-appimage", "*.AppImage", "portal-linux-x86_64.AppImage", "Linux x86_64", "Desktop"),
    ("portal-linux-*-deb", "*.deb", "portal-linux-x86_64.deb", "Linux x86_64", "Desktop"),
    ("portal-linux-*-rpm", "*.rpm", "portal-linux-x86_64.rpm", "Linux x86_64", "Desktop"),
    ("portal-mac-universal-dmg", "*.dmg", "portal-macos-universal.dmg", "macOS Universal", "Desktop"),
    ("portal-mac-universal-app-archive", "*.app.tar.gz", "portal-macos-universal.app.tar.gz", "macOS Universal", "Desktop"),
)
OPENSWAP_REPOSITORY = "citadel-foss/openswap"
# Builds kept in changes.json; an install older than the last one sees only the newest changes.
CHANGES_KEPT = 500
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


def pull_request(message):
    """The (number, title) a merged pull request's commit names, or None for any other commit."""
    lines = message.splitlines()
    first = lines[0].strip() if lines else ""
    if first.startswith("Merge pull request #"):
        number = (first.removeprefix("Merge pull request #").split() or [""])[0]
        title = next((line.strip() for line in lines[1:] if line.strip()), first)
    elif first.endswith(")") and " (#" in first:
        title, number = first[:-1].rsplit(" (#", 1)
    else:
        return None
    return (int(number), title) if number.isdigit() else None


def merged(messages):
    """Pull requests named by commits listed oldest first, newest first and each once."""
    seen, changes = set(), []
    for message in reversed(messages):
        change = pull_request(message)
        if change and change[0] not in seen:
            seen.add(change[0])
            changes.append({"number": change[0], "title": change[1]})
    return changes


def extend_changes(previous_changes, previous_build, display_version, portal_sha, openswap_sha,
                   repository, compare):
    """Prepends this build's pull requests to the published history. `compare(repo, base, head)`
    lists the commit messages between two commits, oldest first."""
    builds = previous_changes.get("builds", [])
    newest = builds[0]["version"] if builds else None
    restart = {"builds": [{"version": display_version, "since": display_version,
                           "portal": [], "openswap": []}]}
    if previous_build is None:
        return restart
    # A rerun of a publish that already went out, or of one whose upload stopped partway.
    if previous_build["display_version"] == display_version:
        return previous_changes if newest == display_version else restart
    # History that doesn't end at the published build has a gap; linking past it would claim
    # changes it never listed.
    if newest != previous_build["display_version"]:
        builds = []

    def since(repo, base, head):
        return [] if base == head else merged(compare(repo, base, head))

    entry = {
        "version": display_version,
        "since": previous_build["display_version"],
        "portal": since(repository, previous_build["portal_sha"], portal_sha),
        "openswap": since(OPENSWAP_REPOSITORY, previous_build["openswap_sha"], openswap_sha),
    }
    return {"builds": [entry, *builds][:CHANGES_KEPT]}


def published(repository, tag, name, empty):
    url = f"https://github.com/{repository}/releases/download/{tag}/{name}"
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return empty
        raise


def compare(repo, base, head):
    output = subprocess.run(
        ["gh", "api", "--paginate", f"repos/{repo}/compare/{base}...{head}?per_page=100",
         "--jq", ".commits[].commit.message | @json"],
        check=True, capture_output=True, text=True,
    ).stdout
    return [json.loads(line) for line in output.splitlines() if line]


def describe(output, assets, repository, tag, openswap_sha, portal_sha, run_url, version, display_version=None):
    base = f"https://github.com/{repository}/releases/download/{tag}"
    entries = []
    for name, platform, kind in assets:
        checksum = hashlib.sha256((output / name).read_bytes()).hexdigest()
        entries.append({"name": name, "platform": platform, "kind": kind,
                        "sha256": checksum, "url": f"{base}/{name}"})
    manifest = {
        "openswap_sha": openswap_sha, "portal_sha": portal_sha, "portal_version": version,
        "display_version": display_version or version,
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
        f"- App version: `{display_version or version}`",
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
    tag = os.environ["RELEASE_TAG"]
    display_version = Path("version.txt").read_text().strip()
    try:
        changes = extend_changes(
            published(repository, tag, "changes.json", {}), published(repository, tag, "build.json", None),
            display_version, os.environ["PORTAL_SHA"], os.environ["OPENSWAP_SHA"], repository, compare,
        )
    # The changelog is optional; downloads still publish, with a history that starts here.
    except Exception as error:
        print(f"::warning::changes.json restarted: {error}")
        changes = extend_changes({}, None, display_version, None, None, repository, compare)
    (output / "changes.json").write_text(json.dumps(changes, indent=1) + "\n")
    notes = describe(output, assets, repository, tag, os.environ["OPENSWAP_SHA"],
                     os.environ["PORTAL_SHA"], run_url, version, display_version)
    Path("release-notes.md").write_text(notes)
    print(notes)
