# SPDX-License-Identifier: Apache-2.0
"""Check publishable paths, source headers, project notices and Markdown indexes."""

import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"
SPDX = "SPDX-License-Identifier: Apache-2.0"
REQUIRED = {
    "LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md", "TRADEMARK.md", "SUPPORT.md",
    "SECURITY.md", "CONTRIBUTING.md", "CODE_OF_CONDUCT.md", "AGENTS.md", "CLAUDE.md",
}
LOCAL_DIRS = {"models", "model-cache", "private-corpus", "receipts-local", "scratch"}
PRIVATE_SUFFIXES = {".pem", ".key", ".pfx", ".p12", ".safetensors", ".onnx", ".gguf", ".pth"}
LOCAL_PATH = re.compile(r"(?:[A-Za-z]:[\\/](?:Users|code|workspaces)[\\/]|/(?:Users|home)/[^/\s]+/)")
LINK = re.compile(r'!?\[[^\]]*\]\(<?([^)>\s]+)>?(?:\s+"[^"]*")?\)')
FENCE = re.compile(r"^\s*(`{3,}|~{3,})(.*)$")


def links_in(text):
    fence = None
    for line_no, line in enumerate(text.splitlines(), 1):
        marker = FENCE.match(line)
        if marker:
            run, suffix = marker.groups()
            if fence is None:
                fence = run
            elif run[0] == fence[0] and len(run) >= len(fence) and not suffix.strip():
                fence = None
            continue
        if fence is None:
            for match in LINK.finditer(line):
                target = urlsplit(match[1])
                if target.path and not (target.scheme or target.netloc):
                    yield line_no, unquote(target.path)


def check_docs(paths, findings):
    markdown = {p for p in paths if p.suffix == ".md" and (p.parent == ROOT or DOCS in p.parents)}
    indexed = set()
    for path in sorted(markdown):
        for line, link in links_in(path.read_text(encoding="utf-8")):
            resolved = (path.parent / link).resolve()
            if not resolved.is_relative_to(ROOT) or resolved not in paths and not (
                resolved.is_dir() and any(resolved in candidate.parents for candidate in paths)
            ):
                findings.append(f"{path.relative_to(ROOT)}:{line}: broken relative link")
            elif path.name == "README.md" and path.parent in resolved.parents:
                indexed.add(resolved)
    for path in sorted(markdown):
        if DOCS not in path.parents:
            continue
        if path.parent / "README.md" not in markdown:
            findings.append(f"{path.parent.relative_to(ROOT)}: missing README.md index")
        if path not in indexed:
            findings.append(f"{path.relative_to(ROOT)}: not linked from an ancestor index")


def main():
    listing = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT,
    )
    findings = []
    paths = set()
    for name in set(listing.decode("utf-8").split("\0")) - {""}:
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            findings.append(f"{name}: expected regular file")
            continue
        paths.add(path)
        if LOCAL_DIRS.intersection(path.relative_to(ROOT).parts[:-1]) or (
            path.suffix.lower() in PRIVATE_SUFFIXES
            or path.name == ".env"
            or path.name.startswith(".env.")
            or path.name.endswith(".local.toml")
        ):
            findings.append(f"{name}: local credential, model or evidence path")
        if path.suffix in {".rs", ".py", ".md", ".toml", ".json", ".yml", ".yaml"}:
            text = path.read_text(encoding="utf-8")
            if LOCAL_PATH.search(text):
                findings.append(f"{name}: local absolute path")
            if path.suffix in {".rs", ".py"}:
                lines = text.splitlines()
                header = lines[1:2] if text.startswith("#!") else lines[:1]
                if not header or header[0] not in {f"# {SPDX}", f"// {SPDX}"}:
                    findings.append(f"{name}: missing Apache-2.0 SPDX header")
    for name in sorted(REQUIRED):
        if ROOT / name not in paths or not (ROOT / name).stat().st_size:
            findings.append(f"{name}: required governance file is missing or empty")
    if {ROOT / "AGENTS.md", ROOT / "CLAUDE.md"} <= paths and (
        (ROOT / "AGENTS.md").read_bytes() != (ROOT / "CLAUDE.md").read_bytes()
    ):
        findings.append("AGENTS.md and CLAUDE.md differ")
    check_docs(paths, findings)
    print("\n".join(findings) if findings else f"Repository hygiene passed for {len(paths)} files.")
    return int(bool(findings))


if __name__ == "__main__":
    sys.exit(main())
