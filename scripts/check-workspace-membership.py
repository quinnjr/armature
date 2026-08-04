#!/usr/bin/env python3
"""Fail if a crate directory is in neither `members` nor `exclude`.

Cargo is silent about a directory it was never told to care about. A crate that
is in neither array is not a workspace member, so the whole-workspace build,
test and clippy runs skip it, and `scripts/publish.sh` — which walks `members` —
never publishes it. Nothing reports this; the crate simply stops being covered.

That has happened three times in this repo:

* `armature-h1` was missing from `members`, so the publish script never saw it.
  `armature-core` requires it, so publishing core would have failed against a
  version of h1 that was not on crates.io.
* `armature-fuzz` stayed a submodule after its targets moved into
  `armature-core/fuzz`, and quietly accumulated failing CI.
* `armature-session` reached version 0.3.0 without ever being built, tested,
  linted or published. Adding it to `members` immediately surfaced two clippy
  denials that had been there the whole time.

The check runs from the workspace root and reports every crate directory cargo
does not account for, plus every `members` entry whose directory has gone.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path


def workspace_arrays(manifest: Path) -> tuple[set[str], set[str]]:
    """The literal `members` and `exclude` entries from the root manifest.

    Parsed textually rather than with a TOML library so the script has no
    dependency to install in CI. Only the two arrays are read, and both hold
    plain path strings, so the shapes a real parser would handle better —
    nested tables, multi-line strings — cannot appear here.
    """
    text = manifest.read_text()

    def array(name: str) -> set[str]:
        m = re.search(rf"^{name} = \[(.*?)\]", text, re.S | re.M)
        if not m:
            return set()
        return {v.rstrip("/") for v in re.findall(r'"([^"]+)"', m.group(1))}

    return array("members"), array("exclude")


def cargo_packages(root: Path) -> set[Path]:
    """Directories cargo reports as workspace packages."""
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        print("cargo metadata failed:\n" + out.stderr, file=sys.stderr)
        raise SystemExit(2)
    meta = json.loads(out.stdout)
    return {Path(p["manifest_path"]).parent.resolve() for p in meta["packages"]}


def declares_own_workspace(manifest: Path) -> bool:
    """Whether the crate opts out by declaring a workspace of its own.

    The `<crate>/fuzz` crates do exactly this: they are nightly-only and must
    stay out of the root workspace, which is a deliberate exclusion rather than
    an oversight.
    """
    return re.search(r"^\[workspace\]", manifest.read_text(), re.M) is not None


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    manifest = root / "Cargo.toml"
    members, exclude = workspace_arrays(manifest)

    # Checked before `cargo metadata`, which refuses to run at all when a member
    # is missing and says only "failed to read <path>". Naming the stale entry
    # and why it is usually stale is more use than that, and running cargo first
    # would make this branch unreachable.
    missing = sorted(
        name for name in members if not (root / name / "Cargo.toml").is_file()
    )
    if missing:
        print("These `members` entries have no Cargo.toml:\n")
        for name in missing:
            print(f"  {name}")
        print(
            "\nA submodule removed without updating `members`, or a checkout\n"
            "without `submodules: recursive`. Cargo cannot load the workspace\n"
            "until this is resolved."
        )
        return 1

    packages = cargo_packages(root)

    unlisted: list[str] = []
    for child in sorted(root.glob("armature-*")):
        if not child.is_dir():
            continue
        crate_manifest = child / "Cargo.toml"
        if not crate_manifest.is_file():
            # An empty submodule directory — checkout without `submodules:
            # recursive`. Not this check's business, and flagging it would make
            # the check fail for a reason unrelated to membership.
            continue
        name = child.name
        if child.resolve() in packages or name in members or name in exclude:
            continue
        if declares_own_workspace(crate_manifest):
            continue
        unlisted.append(name)

    if not unlisted:
        print(f"OK: {len(packages)} packages, every armature-* directory accounted for.")
        return 0

    if unlisted:
        print("These crate directories are in neither `members` nor `exclude`:\n")
        for name in unlisted:
            print(f"  {name}")
        print(
            "\nCargo ignores them: they are never built, tested, linted or\n"
            "published, and nothing else reports it. Add each to `members`, or\n"
            "to `exclude` if it is deliberately outside the workspace."
        )

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
