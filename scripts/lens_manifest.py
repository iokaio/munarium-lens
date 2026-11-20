# SPDX-License-Identifier: Apache-2.0
"""Write a Lens artifact manifest for a local model directory.

Hashes the listed files, writes `lens-manifest.json` into the directory and
prints the manifest's SHA-256 for LENS_<STAGE>_MANIFEST_SHA256. Reads only
local files and never downloads. Check the upstream revision and license
yourself before writing a manifest; this tool does not verify them.

Example:
    python scripts/lens_manifest.py --stage embedding \\
        --served-model qwen3-embedding-0.6b-f32-1024d \\
        --repository Qwen/Qwen3-Embedding-0.6B \\
        --revision 97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3 /srv/lens/embedding
"""

import argparse
import hashlib
import json
from pathlib import Path

FORMAT = "munarium-lens-manifest/0"
REQUIRED = ["config.json", "tokenizer.json", "model.safetensors"]
DIMENSIONS = {"embedding": 1024, "reranking": None}


def digest(path):
    sha = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            sha.update(block)
    return sha.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("directory", type=Path)
    parser.add_argument("--stage", choices=sorted(DIMENSIONS), required=True)
    parser.add_argument("--served-model", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--extra", action="append", default=[],
                        help="another relative file to verify, such as LICENSE")
    args = parser.parse_args()

    files = []
    for name in REQUIRED + args.extra:
        path = args.directory / name
        if path.is_symlink() or not path.is_file():
            parser.error(f"{name}: expected a regular file in the directory")
        files.append({"path": name, "size": path.stat().st_size, "sha256": digest(path)})
    manifest = {
        "format": FORMAT,
        "stage": args.stage,
        "backend": "qwen3",
        "served_model": args.served_model,
        "precision": "f32",
    }
    if DIMENSIONS[args.stage] is not None:
        manifest["dimensions"] = DIMENSIONS[args.stage]
    manifest["upstream"] = {"repository": args.repository, "revision": args.revision}
    manifest["files"] = files
    data = (json.dumps(manifest, indent=2) + "\n").encode("utf-8")
    (args.directory / "lens-manifest.json").write_bytes(data)
    print(hashlib.sha256(data).hexdigest())


if __name__ == "__main__":
    main()
