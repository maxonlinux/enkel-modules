#!/usr/bin/env python3
"""Generate an atomic GitHub catalog update after all verified native release assets have been published."""
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def main():
    """Check complete release metadata and replace only its architectures in the official catalog."""
    directory = Path(sys.argv[1])
    records = [json.loads(path.read_text()) for path in sorted(directory.glob("*.json"))]
    identities = {(entry["manifest"]["id"], entry["manifest"]["architecture"]) for entry in records}
    expected = {(f"dev.enkel.{tool}", arch) for tool in ("console", "network", "elements") for arch in ("aarch64", "x86_64")}
    # Partial releases must never remove working catalog entries for another capability or architecture.
    if identities != expected or len(records) != len(expected):
        raise ValueError("Expected exactly three modules for each supported architecture")
    for record in records:
        archive = directory / record["url"].rsplit("/", 1)[1]
        with archive.open("rb") as source:
            value = hashlib.sha256()
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                value.update(chunk)
            checksum = value.hexdigest()
        # Artifact transfer must preserve the verified archive bytes and their bounded metadata.
        if checksum != record["sha256"] or archive.stat().st_size != record["bytes"]:
            raise ValueError(f"Release artifact changed: {archive}")
    result = subprocess.run(["gh", "api", "repos/maxonlinux/enkel-modules/contents/catalog.json"], check=True, capture_output=True, text=True)
    previous = json.loads(result.stdout)
    catalog = json.loads(base64.b64decode(previous["content"]))
    catalog["modules"] = [entry for entry in catalog["modules"] if (entry["manifest"]["id"], entry["manifest"]["architecture"]) not in identities] + records
    body = dict(message="Publish module release catalog", sha=previous["sha"],
                content=base64.b64encode((json.dumps(catalog, indent=2) + "\n").encode()).decode())
    Path("catalog-update.json").write_text(json.dumps(body))


if __name__ == "__main__":
    main()
