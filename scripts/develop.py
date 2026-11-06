#!/usr/bin/env python3
"""Build local source modules and install sealed native packages into an explicitly selected browser profile."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

from release import ROOT, TOOLS, bundle, run, seal


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
    """Compile this checkout and install modules with the same account-free seals used by releases."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True, help="Explicit browser profile; use a separate development profile")
    parser.add_argument("--module", choices=TOOLS, action="append")
    args = parser.parse_args()
    # Never rewrite a library which a running AppKit process may still have registered classes from.
    if subprocess.run(["/usr/bin/pgrep", "-x", "Enkel"], stdout=subprocess.DEVNULL).returncode != 1:
        parser.error("Quit Enkel normally before replacing development modules")
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
            seal(path)
            for file in [path, *path.rglob("*")]:
                # Local development must not remove or reinterpret quarantine from downloaded executables.
                if "com.apple.quarantine" in run("/usr/bin/xattr", str(file), capture=True).splitlines():
                    raise RuntimeError(f"Expected a fresh local build, found quarantined output: {file}")
            paths.append(path)
        directory = args.profile.resolve() / "Modules"
        directory.mkdir(parents=True, exist_ok=True)
        for path in paths:
            install(path, directory)
    print("Installed native modules. Launch Enkel with the selected profile.")


if __name__ == "__main__":
    main()
