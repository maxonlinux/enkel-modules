#!/usr/bin/env python3
"""Generate an atomic GitHub catalog update after all verified native release assets have been published."""
import base64
import json
from pathlib import Path
import subprocess
import sys

from release import ROOT, digest


def main():
    """Check complete release metadata and replace only its architectures in the official catalog."""
    directory = Path(sys.argv[1])
    records = [json.loads(path.read_text()) for path in sorted(directory.glob("*.json"))]
    identities = {(entry["manifest"]["id"], entry["manifest"]["architecture"]) for entry in records}
    expected = {(f"dev.enkel.{tool}", arch) for tool in ("console", "network", "elements") for arch in ("aarch64", "x86_64")}
    # Partial releases must never remove working catalog entries for another capability or architecture.
    downloads = {("dev.enkel.downloads", arch) for arch in ("aarch64", "x86_64")}
    if identities not in (expected, downloads) or len(records) != len(identities):
        raise ValueError("Expected one verified package per requested capability and architecture")
    for record in records:
        archive = directory / record["url"].rsplit("/", 1)[1]
        # Artifact transfer must preserve the verified archive bytes and their bounded metadata.
        if digest(archive) != record["sha256"] or archive.stat().st_size != record["bytes"]:
            raise ValueError(f"Release artifact changed: {archive}")
    result = subprocess.run(["gh", "api", "repos/maxonlinux/enkel-modules/contents/catalog.json"], check=True, capture_output=True, text=True)
    previous = json.loads(result.stdout)
    catalog = json.loads(base64.b64decode(previous["content"]))
    v2 = ROOT / "catalog-v2.json"
    if v2.exists():
        catalog_v2 = json.loads(v2.read_text())
    else:
        catalog_v2 = dict(schema=1, modules=catalog["modules"])
    def lane(entry):
        manifest = entry["manifest"]
        return (manifest["id"], manifest["architecture"], manifest["abi"], manifest["host"])
    updated = {lane(entry) for entry in records}
    catalog_v2["modules"] = [entry for entry in catalog_v2["modules"] if lane(entry) not in updated] + records
    if len(catalog_v2["modules"]) > 128:
        raise ValueError("Retire unsupported catalog entries before publishing")
    v2.write_text(json.dumps(catalog_v2, indent=2) + "\n")
    if identities == downloads:
        return
    catalog["modules"] = [entry for entry in catalog["modules"] if (entry["manifest"]["id"], entry["manifest"]["architecture"]) not in identities] + records
    body = dict(message="Publish module release catalog", sha=previous["sha"],
                content=base64.b64encode((json.dumps(catalog, indent=2) + "\n").encode()).decode())
    Path("catalog-update.json").write_text(json.dumps(body))


if __name__ == "__main__":
    main()
