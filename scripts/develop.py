#!/usr/bin/env python3
"""Build local source modules and install them into an explicitly selected development profile without an Apple paid account."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from release import ROOT, TOOLS, bundle, run


def install(path, directory):
    """Publish a complete locally signed package using the browser installer's recoverable transaction names."""
    identity = path.stem
    stage = directory / f".install-{identity}"
    backup = directory / f".previous-{identity}"
    target = directory / path.name
    # Let the browser recover an interrupted operation before accepting another source build.
    if stage.exists() or backup.exists():
        raise RuntimeError(f"Interrupted installation for {identity}; launch and quit the browser to recover it")
    shutil.copytree(path, stage)
    try:
        # Retain the previous complete installation until the prepared package can take its canonical path.
        if target.exists():
            target.rename(backup)
        stage.rename(target)
    except OSError:
        # Failed publication must not leave a previously installed package missing.
        if backup.exists() and not target.exists():
            backup.rename(target)
        raise
    finally:
        # Only private incomplete staging files are disposable after a failed transaction.
        if stage.exists():
            shutil.rmtree(stage)
    # Publication has committed; removing the retained backup cannot expose a partial new package.
    if backup.exists():
        shutil.rmtree(backup)


def main():
    """Compile this checkout, sign it with the developer's existing certificate and install into an offline development profile."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True, help="Explicit browser profile; use a separate development profile")
    parser.add_argument("--identity", required=True, help="Certificate SHA-1 also declared by the local browser debug build")
    parser.add_argument("--module", choices=TOOLS, action="append")
    args = parser.parse_args()
    # Never rewrite a library which a running AppKit process may still have registered classes from.
    if subprocess.run(["/usr/bin/pgrep", "-x", "Enkel"], stdout=subprocess.DEVNULL).returncode != 1:
        parser.error("Quit Enkel normally before replacing development modules")
    # A real local developer certificate provides stable ownership without pretending to be a notarized public release.
    if not re.fullmatch(r"[A-Fa-f0-9]{40}", args.identity):
        parser.error("Expected a code-signing certificate SHA-1; ad-hoc signatures are not accepted")
    metadata = json.loads(run("cargo", "metadata", "--locked", "--no-deps", "--format-version", "1", capture=True))
    version = next(p["version"] for p in metadata["packages"] if p["name"] == "enkel-module-sdk")
    target = next(line.split(": ", 1)[1] for line in run("rustc", "-vV", capture=True).splitlines() if line.startswith("host: "))
    modules = list(dict.fromkeys(args.module or TOOLS))
    output = ROOT / "dist"
    output.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="development-", dir=output) as temporary:
        work = Path(temporary)
        paths = []
        for tool in modules:
            run("cargo", "build", "--locked", "--release", "--target", target, "-p", f"enkel-module-{tool}")
            path, _ = bundle(tool, target, version, work)
            run("/usr/bin/codesign", "--force", "--timestamp=none", "--sign", args.identity, str(path))
            requirement = f'=certificate leaf = H"{args.identity}" and identifier = "{path.stem}"'
            run("/usr/bin/codesign", "--verify", "--strict", "-R", requirement, str(path))
            for file in [path, *path.rglob("*")]:
                # Local development must not remove or reinterpret quarantine from downloaded executables.
                if "com.apple.quarantine" in os.listxattr(file):
                    raise RuntimeError(f"Expected a fresh local build, found quarantined output: {file}")
            paths.append(path)
        directory = args.profile.resolve() / "Modules"
        directory.mkdir(parents=True, exist_ok=True)
        for path in paths:
            install(path, directory)
    print("Installed local source modules. Launch the local debug browser with ENKEL_DEVELOPMENT_MODULES=1 and this profile.")


if __name__ == "__main__":
    main()
