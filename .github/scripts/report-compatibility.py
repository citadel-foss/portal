#!/usr/bin/env python3
"""Report failed platforms with focused compiler output and the full job logs."""

import json
import os
from pathlib import Path
import re
import subprocess
import time


ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)")
TIMESTAMP = re.compile(r"^\ufeff?\d{4}-\d{2}-\d{2}T\S+\s+")
ERROR = re.compile(
    r"^\s*(?:error(?:\[[^]]+\])?:|npm (?:err!|error)|fatal:|caused by:|##\[error\])"
    r"|(?:\(\d+,\d+\)|:\d+:\d+):?\s*error\b", re.IGNORECASE
)


def gh(*args):
    result = subprocess.run(["gh", *args], text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return result.stdout


def error_excerpt(log):
    lines = [TIMESTAMP.sub("", line) for line in ANSI.sub("", log).replace("\r", "").splitlines()]
    start = next((i for i, line in enumerate(lines)
                  if ERROR.search(line) and "Process completed with exit code" not in line), None)
    if start is None:
        summary = "Build command failed; see the job log for details"
        excerpt = "\n".join(lines[-60:])
    else:
        summary = lines[start].strip()
        excerpt = "\n".join(lines[start:start + 80])
    return summary, excerpt[:6000].replace("```", "` ` `")


def report():
    repository = os.environ["GITHUB_REPOSITORY"]
    run = os.environ["GITHUB_RUN_ID"]
    attempt = os.environ["GITHUB_RUN_ATTEMPT"]
    sha = os.environ["OPENSWAP_SHA"]
    portal_sha = os.environ["PORTAL_SHA"]
    run_url = f"https://github.com/{repository}/actions/runs/{run}"
    marker = f"<!-- openswap-compatibility:{sha} -->"
    output = Path("compatibility-report")
    output.mkdir(exist_ok=True)
    pages = json.loads(gh("api", "--paginate", "--slurp",
                         f"repos/{repository}/actions/runs/{run}/attempts/{attempt}/jobs?per_page=100"))
    failed = [job for page in pages for job in page["jobs"]
              if job["conclusion"] in ("failure", "timed_out")]
    if not failed:
        raise RuntimeError("CI reported failure but the Actions API returned no failed jobs")
    body = [marker, "Portal could not build or publish against the selected OpenSwap master commit.", "",
            f"- OpenSwap: [{sha}](https://github.com/citadel-foss/openswap/commit/{sha})",
            f"- Portal: [{portal_sha}](https://github.com/{repository}/commit/{portal_sha})",
            f"- [Workflow run and full logs]({run_url})", ""]
    summaries = []
    for job in failed:
        step = next((step["name"] for step in job["steps"] if step["conclusion"] == "failure"),
                    "Job timed out or failed before completing a step")
        # Completed job logs can briefly lag behind the job status.
        for retry in range(3):
            try:
                log = gh("api", f"repos/{repository}/actions/jobs/{job['id']}/logs")
                break
            except RuntimeError as error:
                log = f"Job log is not available yet: {error}\nFull logs: {job['html_url']}"
                if retry < 2:
                    time.sleep(2)
        (output / f"job-{job['id']}.log").write_text(log)
        summary, excerpt = error_excerpt(log)
        summaries.append(summary)
        body.extend([f"### {job['name']}", "", f"Failed step: **{step}**", "",
                     f"[Complete job log]({job['html_url']})", "", "```text", excerpt, "```", ""])
    issue_body = output / "issue.md"
    issue_body.write_text("\n".join(body))
    summary = re.sub(r"[\x00-\x1f`*_<>]", "", summaries[0])
    title = f"Portal build failed against OpenSwap {sha[:12]}: {summary[:110]}"
    issue_pages = json.loads(gh("api", "--paginate", "--slurp",
                               f"repos/{repository}/issues?state=open&per_page=100"))
    existing = next((issue for page in issue_pages for issue in page
                     if "pull_request" not in issue and marker in (issue.get("body") or "")), None)
    if existing:
        number = str(existing["number"])
        gh("issue", "edit", number, "--repo", repository, "--title", title,
           "--body-file", str(issue_body))
        comment = output / "rerun.md"
        comment.write_text(f"Compatibility still fails on [run {run}, attempt {attempt}]({run_url}). "
                           "The issue body has been updated with the latest errors.\n")
        gh("issue", "comment", number, "--repo", repository, "--body-file", str(comment))
        issue_url = existing["html_url"]
    else:
        issue_url = gh("issue", "create", "--repo", repository, "--title", title,
                       "--body-file", str(issue_body)).strip()
    print(f"Compatibility failure reported: {issue_url}")
    with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as summary_file:
        summary_file.write(f"Compatibility failure: [{title}]({issue_url}).\n")


if __name__ == "__main__":
    report()
