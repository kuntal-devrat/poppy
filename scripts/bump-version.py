#!/usr/bin/env python3
"""
Automated Version Updation Script for Poppy Workspace.
Bumps [workspace.package] version, updates all internal [workspace.dependencies] in Cargo.toml,
and regenerates Cargo.lock across all workspace member crates.

Supports automatic semver detection based on Conventional Commits since the last release tag:
- 'BREAKING CHANGE:' or '!:' -> MAJOR bump
- 'feat:' or 'feat(...):' -> MINOR bump
- 'fix:', 'perf:', 'refactor:', etc. -> PATCH bump
"""

import os
import re
import subprocess
import sys
from pathlib import Path


def get_workspace_root() -> Path:
    # Root is the parent directory of this scripts/ directory
    return Path(__file__).resolve().parent.parent


def parse_current_version(cargo_toml_content: str) -> str:
    match = re.search(
        r'\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"',
        cargo_toml_content,
    )
    if not match:
        raise ValueError("Could not find [workspace.package] version in Cargo.toml")
    return match.group(1)


def detect_bump_type_from_git(root: Path) -> str:
    """
    Analyzes git commit messages since the most recent git tag.
    - If any commit has a BREAKING CHANGE or '!:' suffix: returns 'major'
    - If any commit starts with 'feat': returns 'minor'
    - Otherwise (e.g. 'fix', 'perf', 'refactor', etc.): returns 'patch'
    """
    try:
        tag_proc = subprocess.run(
            ["git", "describe", "--tags", "--abbrev=0"],
            cwd=root,
            capture_output=True,
            text=True,
        )
        if tag_proc.returncode == 0 and tag_proc.stdout.strip():
            latest_tag = tag_proc.stdout.strip()
            rev_range = f"{latest_tag}..HEAD"
        else:
            latest_tag = "initial"
            rev_range = "HEAD"

        log_proc = subprocess.run(
            ["git", "log", rev_range, "--pretty=format:%B%x1f"],
            cwd=root,
            capture_output=True,
            text=True,
        )
        messages = [m.strip() for m in log_proc.stdout.split("\x1f") if m.strip()]
        if not messages:
            print(f"ℹ️ No new commits detected since tag '{latest_tag}'.")
            return "none"

        has_breaking = False
        has_feat = False

        for msg in messages:
            lines = msg.splitlines()
            first_line = lines[0].strip() if lines else ""
            if "BREAKING CHANGE" in msg or "BREAKING-CHANGE" in msg:
                has_breaking = True
                break
            # Conventional commit breaking change: type!: or type(scope)!:
            if re.match(r"^[a-zA-Z0-9_-]+(\([^)]+\))?!:", first_line):
                has_breaking = True
                break
            if re.match(r"^feat(\([^)]+\))?:", first_line):
                has_feat = True

        if has_breaking:
            print(f"🔍 Analyzed {len(messages)} commits since {latest_tag}: Found BREAKING CHANGE -> Bumping MAJOR")
            return "major"
        elif has_feat:
            print(f"🔍 Analyzed {len(messages)} commits since {latest_tag}: Found new feature ('feat:') -> Bumping MINOR")
            return "minor"
        else:
            print(f"🔍 Analyzed {len(messages)} commits since {latest_tag}: Found fixes / maintenance -> Bumping PATCH")
            return "patch"
    except Exception as e:
        print(f"⚠️ Git history inspection failed ({e}), defaulting to patch bump.")
        return "patch"


def compute_new_version(root: Path, current_version: str, bump_type: str, custom_version: str = "") -> str:
    if custom_version and custom_version.strip():
        val = custom_version.strip().lstrip("v")
        if not re.match(r"^\d+\.\d+\.\d+", val):
            raise ValueError(f"Invalid custom semver version: {custom_version}")
        return val

    bump = bump_type.lower().strip()
    if bump == "auto":
        bump = detect_bump_type_from_git(root)
        if bump == "none":
            return current_version

    match = re.match(r"^(\d+)\.(\d+)\.(\d+)(.*)$", current_version)
    if not match:
        raise ValueError(f"Current version '{current_version}' is not valid semver")

    major, minor, patch = int(match.group(1)), int(match.group(2)), int(match.group(3))
    prerelease = match.group(4)

    if bump == "major":
        return f"{major + 1}.0.0"
    elif bump == "minor":
        return f"{minor + 1}.0.0" if prerelease else f"{major}.{minor + 1}.0"
    elif bump in ("patch", "fix"):
        return f"{major}.{minor}.{patch + 1}"
    else:
        # Check if bump_type was directly a version string
        val = bump.lstrip("v")
        if re.match(r"^\d+\.\d+\.\d+", val):
            return val
        raise ValueError(
            f"Unknown bump type: '{bump_type}'. Expected 'auto', 'patch', 'minor', 'major', or a semver string."
        )


def update_cargo_toml(root: Path, new_version: str) -> None:
    cargo_path = root / "Cargo.toml"
    content = cargo_path.read_text(encoding="utf-8")

    # 1. Update [workspace.package] version
    pkg_pattern = r'(\[workspace\.package\][\s\S]*?version\s*=\s*)"[^"]+"'
    if not re.search(pkg_pattern, content):
        raise ValueError("Failed to match [workspace.package] version")
    content = re.sub(pkg_pattern, rf'\g<1>"{new_version}"', content, count=1)

    # 2. Update all internal poppy-* workspace dependencies:
    # e.g. poppy-core = { version = "0.6.1", path = "crates/poppy-core" }
    dep_pattern = r'((?:poppy|poppy-[a-z0-9_-]+)\s*=\s*\{\s*version\s*=\s*)"[^"]+"'
    content = re.sub(dep_pattern, rf'\g<1>"{new_version}"', content)

    cargo_path.write_text(content, encoding="utf-8")
    print(f"✅ Updated Cargo.toml to version {new_version}")


def sync_cargo_lock(root: Path) -> None:
    print("🔄 Running `cargo check --workspace` to update Cargo.lock...")
    res = subprocess.run(
        ["cargo", "check", "--workspace"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if res.returncode != 0:
        print(f"❌ Error updating Cargo.lock:\n{res.stderr}", file=sys.stderr)
        sys.exit(1)
    print("✅ Cargo.lock successfully synchronized with all workspace crates.")


def export_github_output(new_version: str, previous_version: str) -> None:
    output_file = os.environ.get("GITHUB_OUTPUT")
    if output_file:
        with open(output_file, "a", encoding="utf-8") as f:
            f.write(f"new_version={new_version}\n")
            f.write(f"previous_version={previous_version}\n")
            f.write(f"tag=v{new_version}\n")
            f.write(f"release_title=v{new_version}\n")
            f.write(f"has_changed={'true' if new_version != previous_version else 'false'}\n")
        print(f"📤 Exported GitHub Actions output: v{new_version}")


def main():
    root = get_workspace_root()
    cargo_path = root / "Cargo.toml"
    if not cargo_path.exists():
        print(f"❌ Cargo.toml not found at {cargo_path}", file=sys.stderr)
        sys.exit(1)

    bump_type = sys.argv[1] if len(sys.argv) > 1 else "auto"
    custom_version = sys.argv[2] if len(sys.argv) > 2 else ""

    content = cargo_path.read_text(encoding="utf-8")
    current_version = parse_current_version(content)
    new_version = compute_new_version(root, current_version, bump_type, custom_version)

    print("==================================================")
    print("🌸 Poppy Workspace Version Bumper")
    print(f"Current version:  {current_version}")
    print(f"Bump type:        {bump_type}")
    print(f"Target version:   {new_version}")
    print("==================================================")

    if current_version == new_version:
        print("⚠️ Current version already matches target version. No changes needed.")
    else:
        update_cargo_toml(root, new_version)
        sync_cargo_lock(root)

    export_github_output(new_version, current_version)
    if current_version != new_version:
        print(f"\n🎉 Successfully bumped Poppy workspace to v{new_version}!")
    else:
        print(f"\n✨ Workspace is up to date at v{current_version}.")


if __name__ == "__main__":
    main()
