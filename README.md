# Munarium Lens

Munarium Lens is the planned self-hosted embedding and passage reranking endpoint
for [Munarium Server](https://github.com/iokaio/munarium). It will let operators
place retrieval inference on their own hardware and size it independently of Server.

**Status — LENS-0 scaffold verified. LENS-1: reference backend ready; model
backend in progress.** The Rust service serves readiness, model discovery,
embeddings and reranking with request limits and typed refusals. Its only
backend is a deterministic reference implementation for tests and protocol
checks. It is **not a retrieval model**. On 20 November 2026 the maintainer
accepted Qwen3-Embedding-0.6B with Qwen3-Reranker-0.6B at 1,024 dimensions,
with instruction prefixes owned by Server, and confirmed
[Candle](docs/design/model-runtime.md) as the runtime. The model backend is not
yet qualified, so LENS-1 is not complete. Both stages are disabled and the
listener is loopback-only by default.
The tests need no model weights, GPU, credentials or Server.

```sh
cargo test --locked --offline
LENS_EMBEDDING_BACKEND=reference LENS_RERANKING_BACKEND=reference cargo run --locked --offline
```

## Model and weight distribution

Lens model files are published in two places with the same bytes:

| Location | Role |
|---|---|
| [`huggingface.co/iokaio/munarium-lens-models`](https://huggingface.co/iokaio/munarium-lens-models) | Primary. A Hugging Face model repository holding every model bundle Lens serves. |
| [GitHub Releases of `iokaio/munarium-lens`](https://github.com/iokaio/munarium-lens/releases) | Backup. Each bundle is also attached to a release, so a deployment still works if the primary is unavailable. |

**Status: location decided 20 November 2026; download support planned.** Lens
does not download anything yet. Today an operator fetches a bundle by hand
(see [Fetching a bundle by hand](#fetching-a-bundle-by-hand)) and points
`LENS_EMBEDDING_ARTIFACTS` or `LENS_RERANKING_ARTIFACTS` at it. Everything below
marked *planned* describes the intended behavior, not current code.

### What is published

A **bundle** is one directory per served model. It holds exactly the files a
Lens stage loads, plus provenance:

```text
iokaio/munarium-lens-models
├── README.md                              repository model card: every bundle, its upstream and license
├── qwen3-embedding-0.6b-f32-1024d/
│   ├── lens-manifest.json                 file list, sizes, SHA-256 digests, upstream repository and revision
│   ├── config.json
│   ├── tokenizer.json
│   ├── model.safetensors
│   ├── LICENSE                            the upstream license, unchanged
│   └── README.md                          bundle card: upstream, revision, changes (if any), served name
└── qwen3-reranker-0.6b-f32/
    └── … (same shape; a reranking manifest has no `dimensions`)
```

- The directory name is the manifest's `served_model`. That name is permanent.
  Changing the weights, tokenizer, precision, template or dimensions produces a
  new bundle with a new name. A published bundle is never edited in place.
- The first bundles are the accepted pair: Qwen3-Embedding-0.6B and
  Qwen3-Reranker-0.6B (FP32, 1,024 dimensions), at the upstream revisions pinned
  in the [model runtime decision](docs/design/model-runtime.md).
- **Custom weights** use the same layout. Examples are Munarium-specific
  fine-tunes, converted precisions, or re-exported tokenizers. Their bundle card
  must state the upstream base and revision, exactly what changed and how, the
  evaluation evidence, and the license. Custom weights are published only after
  the [qualification](docs/design/qualification.md) the change requires.
- No pickle files (`*.bin`, `*.pt`, `*.pth`) are ever published. Weights are
  `safetensors` only.

### How Lens identifies a bundle

A deployment pins three values and nothing else is trusted:

1. the bundle name, for example `qwen3-embedding-0.6b-f32-1024d`;
2. the Hugging Face repository **commit** that contains it: the full 40-character
   revision, never a branch name such as `main`;
3. the SHA-256 of the bundle's `lens-manifest.json`. This is the digest
   `LENS_*_MANIFEST_SHA256` already takes.

The manifest digest is the security boundary. Lens verifies every listed file's
size and SHA-256 against the manifest, and the manifest against the pinned digest,
before loading anything (see [artifact verification](docs/design/qwen3-backend.md#artifact-manifest-and-verification)).
Verification does not depend on where the bytes came from. A bundle fetched from
Hugging Face, from the GitHub release, or copied by hand onto an air-gapped host
passes or fails identically. A file that differs from the manifest leaves the
stage `failed` and readiness at 503.

### GitHub Releases backup

- Each bundle is attached to a release in this repository. The tag is
  `models/<served_model>`, for example `models/qwen3-embedding-0.6b-f32-1024d`.
  Every file is a release asset under its bundle-relative name, including
  `lens-manifest.json`, `LICENSE` and the bundle card.
- The release notes record the Hugging Face commit the bundle was published
  from and the manifest SHA-256, so the two locations can be checked against
  each other.
- GitHub limits a release asset to 2 GiB. The current bundles fit: the largest
  file is about 1.2 GB. A larger model is published as sharded `safetensors`,
  each shard listed in the manifest and below the limit.
- Release assets are immutable once published, matching the rule that a bundle
  is never edited. A corrected bundle gets a new name and a new tag.

### Downloading (planned)

Planned behavior, labelled so until implemented:

- **Off by default.** With no download setting, Lens behaves exactly as it does
  today: it loads only from the operator-supplied artifact directory and makes
  no outbound connection.
- **Explicit, before serving.** Downloads happen through an explicit fetch step
  (a `munarium-lens fetch` subcommand), or at startup when the operator enables
  it. They never happen while a request is being served. A stage stays `failed`
  until its bundle is present and verified.
- **Source order.** Hugging Face first, then the GitHub release, configurable.
  Lens fetches by the pinned commit and bundle name. It never follows a branch,
  a `latest` tag or a redirect to an unexpected host.
- **Safe writes.** Each file streams into a temporary file in the target
  directory, is checked against the manifest's size and SHA-256, and is then
  renamed into place. A partial or wrong download is discarded and never loaded.
  Files already present and verified are not fetched again.
- **No credentials needed.** Both locations are public. Lens neither requires
  nor logs a token, and logs no file paths or values on a verification failure.
- **Network surface.** Fetching needs outbound HTTPS to `huggingface.co` (and
  the Hugging Face CDN hosts it redirects to) or to `github.com` (and the GitHub
  release-asset host). Standard proxy settings apply. A serving Lens with
  downloads off needs no egress.
- **Offline and air-gapped hosts.** Fetch on a connected host, copy the bundle
  directory, and point `LENS_*_ARTIFACTS` at it. The pinned manifest digest
  proves the copy is intact.
- **Dependency policy.** A built-in fetcher needs an HTTP client and a TLS
  stack, which the current [`deny.toml`](deny.toml) bans on purpose for the
  serving build. The fetcher will therefore sit behind its own cargo feature,
  off by default, and its policy exception needs maintainer review. The default
  serving image stays free of HTTP clients and TLS.

### Fetching a bundle by hand

Until download support lands, use either location directly, then pin the digest
that `lens-manifest.json` produces:

```sh
# From Hugging Face, at a pinned commit (requires the `hf` CLI)
hf download iokaio/munarium-lens-models --revision <40-char-commit> \
  --include "qwen3-embedding-0.6b-f32-1024d/*" --local-dir /srv/lens

# Or from the GitHub release backup (requires the `gh` CLI)
gh release download models/qwen3-embedding-0.6b-f32-1024d \
  --repo iokaio/munarium-lens --dir /srv/lens/qwen3-embedding-0.6b-f32-1024d

# Pin the manifest digest
sha256sum /srv/lens/qwen3-embedding-0.6b-f32-1024d/lens-manifest.json
```

Then set `LENS_EMBEDDING_ARTIFACTS=/srv/lens/qwen3-embedding-0.6b-f32-1024d` and
`LENS_EMBEDDING_MANIFEST_SHA256=<digest>`, and mount the directory read-only.

### Publishing a bundle

Publishing is a maintainer action. It is never done by CI or by an automated
agent without explicit authorization.

1. Download the upstream files at the pinned upstream revision. Check the
   upstream license permits redistribution: the Qwen3 models are Apache-2.0.
2. Write `lens-manifest.json` with `scripts/lens_manifest.py` (it records the
   upstream repository and revision) and add the upstream `LICENSE` and a bundle
   card.
3. Verify the directory by loading it in Lens with the new digest, and run the
   real-weight test.
4. Upload the bundle to `iokaio/munarium-lens-models` and note the resulting
   commit.
5. Create the `models/<served_model>` release with the same files. Record the
   Hugging Face commit and manifest digest in the release notes.
6. Record the bundle, commit and digest in the
   [model runtime decision](docs/design/model-runtime.md) and the CHANGELOG.

The weights keep their upstream license. Munarium Lens's own Apache-2.0 license
covers this repository's code, not the model weights.
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) lists the upstream model
licenses.

## Documentation

See the [local development walkthrough](docs/guides/local-development.md) for
the probe commands, configuration and the reference container. The [design and implementation plan](docs/design-plan.md)
tracks the Server v1.5 dependencies, implementation milestones and release receipts.
The [documentation index](docs/README.md) links the endpoint, artifact, serving and
qualification designs.

[Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) ·
[Support](SUPPORT.md) · [Apache-2.0 license](LICENSE)
