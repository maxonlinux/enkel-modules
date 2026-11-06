#!/usr/bin/env python3
"""Build independent native bundles with account-free code-integrity seals and reproducible release metadata."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ("console", "network", "elements")


def run(*args, capture=False):
    """Execute structured arguments without a shell and stop the release on any failure."""
    result = subprocess.run(args, cwd=ROOT, check=True, text=True,
                            stdout=subprocess.PIPE if capture else None)
    return result.stdout if capture else None


def digest(path):
    """Hash file content in bounded chunks without retaining an entire module image."""
    with path.open("rb") as source:
        value = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
        return value.hexdigest()


def bundle(tool, target, version, destination):
    """Give each independently compiled library its own native bundle and versioned host manifest."""
    identity = f"dev.enkel.{tool}"
    path = destination / f"{identity}.enkelmodule"
    (path / "Contents/MacOS").mkdir(parents=True)
    (path / "Contents/Resources").mkdir()
    binary = path / "Contents/MacOS/Module"
    shutil.copy2(ROOT / f"target/{target}/release/libenkel_module_{tool}.dylib", binary)
    manifest = dict(id=identity, version=version, host="^0.2", abi=1,
                    architecture=target.split("-")[0], build=digest(binary))
    (path / "Contents/Resources/module.json").write_text(json.dumps(manifest, indent=2) + "\n")
    info = dict(CFBundleIdentifier=identity, CFBundleExecutable="Module",
                CFBundleName=tool.title(), CFBundleVersion=version,
                CFBundleShortVersionString=version, CFBundlePackageType="BNDL",
                LSMinimumSystemVersion="13.0")
    subprocess.run(["/usr/bin/plutil", "-convert", "xml1", "-o", str(path / "Contents/Info.plist"), "-"],
                   input=json.dumps(info), text=True, check=True)
    return path, manifest


def seal(path):
    """Apply an ad-hoc resource seal for native code integrity without a certificate or Apple account."""
    run("/usr/bin/codesign", "--force", "--timestamp=none", "--sign", "-", str(path))
    run("/usr/bin/codesign", "--verify", "--strict", str(path))


def publishable(tool, target, tag, path, manifest, output):
    """Archive only ordinary POSIX entries and record the exact bytes the browser must verify."""
    name = f"{tool}-{target.split('-')[0]}.tar"
    archive = output / name
    with tarfile.open(archive, "w", format=tarfile.USTAR_FORMAT) as tar:
        for file in [path, *sorted(path.rglob("*"))]:
            # Signed bundles have no symlink or special-file payload; keep the archive namespace closed.
            if file.is_symlink() or not (file.is_dir() or file.is_file()):
                raise ValueError(f"Unsupported bundle entry: {file}")
            info = tar.gettarinfo(str(file), str(file.relative_to(path.parent)))
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            with file.open("rb") if file.is_file() else open(os.devnull, "rb") as source:
                tar.addfile(info, source if file.is_file() else None)
    release = dict(manifest=manifest,
                   url=f"https://github.com/maxonlinux/enkel-modules/releases/download/{tag}/{name}",
                   sha256=digest(archive), bytes=archive.stat().st_size)
    (output / f"{tool}-{manifest['architecture']}.json").write_text(json.dumps(release, indent=2) + "\n")


def main():
    """Build all modules for one architecture without requiring publisher credentials."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["aarch64-apple-darwin", "x86_64-apple-darwin"], required=True)
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    # Bound release tags before constructing catalog URLs and selecting project versions.
    if not re.fullmatch(r"v\d+\.\d+\.\d+", args.tag):
        parser.error("Expected a vMAJOR.MINOR.PATCH tag")
    version = args.tag[1:]
    metadata = json.loads(run("cargo", "metadata", "--no-deps", "--format-version", "1", capture=True))
    # A tag cannot relabel old compiled project versions as a newer published release.
    if any(p["version"] != version for p in metadata["packages"] if p["name"].startswith("enkel-module-")):
        parser.error("Release tag must match the workspace version")
    output = ROOT / "dist" / args.target
    # Refuse stale artifacts instead of mixing them into a newly approved release.
    if output.exists():
        parser.error(f"Release output already exists: {output}")
    output.mkdir(parents=True)
    work = output / "work"
    work.mkdir()
    for tool in TOOLS:
        run("cargo", "build", "--locked", "--release", "--target", args.target, "-p", f"enkel-module-{tool}")
        path, manifest = bundle(tool, args.target, version, work)
        seal(path)
        publishable(tool, args.target, args.tag, path, manifest, output)
    shutil.rmtree(work)


if __name__ == "__main__":
    main()
