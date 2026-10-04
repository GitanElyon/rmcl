# SPDX-FileCopyrightText: 2026 Constantin Bauer
# SPDX-License-Identifier: GPL-3.0-only

import os
import re
import subprocess
import sys


def version_key(value):
    match = re.fullmatch(
        r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
        r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
        r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?",
        value,
    )
    if match is None:
        return None
    pre = match[4].split(".") if match[4] is not None else []
    if any(part.isdigit() and len(part) > 1 and part.startswith("0") for part in pre):
        return None
    return tuple(map(int, match.groups()[:3])) + (
        match[4] is None,
        tuple((0, int(part)) if part.isdigit() else (1, part) for part in pre),
    )


def check():
    chain = "1.0.0-alpha 1.0.0-alpha.1 1.0.0-alpha.beta 1.0.0-beta 1.0.0-beta.2 1.0.0-beta.11 1.0.0-rc.1 1.0.0".split()
    assert all(version_key(a) < version_key(b) for a, b in zip(chain, chain[1:]))
    assert version_key("1.0.0+build2") == version_key("1.0.0+build1")
    assert all(version_key(v) is None for v in ["01.0.0", "1.0.0-01", "1.0.0-alpha..1"])
    assert version_key("0.5.1") > version_key("0.5.1-rc.1")
    assert version_key("0.5.1-rc.10") > version_key("0.5.1-rc.9")


if __name__ == "__main__":
    if sys.argv[1:] == ["--check"]:
        check()
    else:
        version = sys.argv[1]
        candidate = version_key(version)
        if candidate is None:
            sys.exit(f"Invalid package version: {version}")
        versions = [
            (key, tag[1:])
            for tag in subprocess.check_output(["git", "tag", "--list", "v*"], text=True).splitlines()
            if (key := version_key(tag[1:])) is not None
        ]
        latest_key, latest = max(versions, default=(None, ""))
        release = latest_key is None or candidate > latest_key
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(f"release={str(release).lower()}\ntag=v{version}\n")
        print(f"v{version}: release={str(release).lower()}, latest={latest or 'none'}")
