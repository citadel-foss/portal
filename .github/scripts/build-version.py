#!/usr/bin/env python3
"""Reserve one display version per compatibility run, shared by every platform."""

import base64
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import sys


class GitHubError(RuntimeError):
    pass


def gh(*args, payload=None):
    result = subprocess.run(["gh", *args], text=True, capture_output=True,
                            input=json.dumps(payload) if payload is not None else None)
    if result.returncode:
        raise GitHubError(result.stderr.strip())
    return json.loads(result.stdout)


def build_date():
    return datetime.now(timezone.utc).strftime("%Y%m%d")


def validate_date(day):
    if not re.fullmatch(r"[0-9]{8}", day):
        raise ValueError("Build date must use YYYYMMDD")
    datetime.strptime(day, "%Y%m%d")


def counter(version, base):
    match = re.fullmatch(re.escape(base) + r"\.([1-9][0-9]*)\+([0-9]{8})", version)
    if not match:
        raise ValueError(f"Display version must look like {base}.1+YYYYMMDD")
    validate_date(match[2])
    return int(match[1])


def next_version(version, base, day=None):
    day = day or build_date()
    validate_date(day)
    return f"{base}.{counter(version, base) + 1}+{day}"


def package_version(root):
    return json.loads((root / "src-tauri/tauri.conf.json").read_text())["version"]


def stamp(root, version):
    counter(version, package_version(root))
    (root / "version.txt").write_text(version + "\n")
    return version


def reserve(root, repository, ref):
    base = package_version(root)
    day = build_date()
    if ref != "refs/heads/main":
        # Development branch runs must not advance the public counter on main.
        return stamp(root, next_version((root / "version.txt").read_text().strip(), base, day))
    endpoint = f"repos/{repository}/contents/version.txt"
    for attempt in range(3):
        current = gh("api", endpoint + "?ref=main")
        if current["encoding"] != "base64":
            raise ValueError("Expected a base64-encoded version.txt from GitHub")
        value = base64.b64decode(current["content"]).decode().strip()
        version = next_version(value, base, day)
        payload = {
            "message": f"ci: bump Portal display version to {version}",
            "content": base64.b64encode((version + "\n").encode()).decode(),
            "sha": current["sha"], "branch": "main",
        }
        try:
            gh("api", "--method", "PUT", endpoint, "--input", "-", payload=payload)
        except GitHubError as error:
            # Optimistic locking handles another writer updating the file between GET/PUT.
            if attempt < 2 and any(status in str(error) for status in ("HTTP 409", "HTTP 422")):
                continue
            raise
        return stamp(root, version)
    raise RuntimeError("Could not reserve a display version")


if __name__ == "__main__":
    root = Path.cwd()
    if sys.argv[1:] == ["reserve"]:
        version = reserve(root, os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_REF"])
        with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
            output.write(f"version={version}\n")
    elif sys.argv[1:] == ["stamp"]:
        version = stamp(root, os.environ["PORTAL_BUILD_VERSION"])
    else:
        raise SystemExit("Usage: build-version.py reserve|stamp")
    print(f"Portal display version: {version}")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as summary:
            summary.write(f"Portal display version: `{version}`.\n")
