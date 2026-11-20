# Contributing to Munarium Lens

Contributions are welcome under [Apache-2.0](LICENSE), with a Developer
Certificate of Origin ([DCO](https://developercertificate.org/)) sign-off on
every commit. Use `git commit -s` with your own name and email. The sign-off
certifies your right to submit the contribution under the repository license.

1. Start from current `main`, then use a topic branch or fork.
2. Follow the [local walkthrough](docs/guides/local-development.md) and run
   the checks below. Use synthetic public fixtures. Keep credentials, model
   assets, local configuration and private corpus evidence outside the tree.
3. Open a PR to `main` and complete the disclosure fields for third-party
   code, generated code, AI tooling and employer or contractual restrictions.
   State which checks ran and which coverage remains pending. Leave review
   attestations pending until the human review is complete.
4. A maintainer reviews the actual diff and squashes the PR after required
   checks pass and conversations are resolved. Preserve the human DCO
   trailers and review the final squash message before merging. When the sole
   maintainer authors the PR, record the review in the PR template.

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python scripts/check_repository.py
gitleaks dir . --redact
gitleaks git . --redact
git diff --check
```

Keep `AGENTS.md` and `CLAUDE.md` byte-identical, use the Apache-2.0 SPDX
header on new source files, and index new design or operator pages. The
repository check covers relative Markdown targets and documentation indexes,
source headers, notice presence, guidance parity and likely local artifact
paths. Gitleaks checks for credential patterns. Maintainers review the diff
for private material beyond those patterns.

CI runs without production credentials, with read-only token permissions and
pinned actions. License, notices, marks, contributor/security/conduct/support
policy, `.github/`, and future published contracts and signing/release
configuration are maintained by Ioka. Propose shared wire changes through
the hub decision and contract export process in the [design plan](docs/design-plan.md).
The [Code of Conduct](CODE_OF_CONDUCT.md) and [private security route](SECURITY.md)
apply to contributors.
