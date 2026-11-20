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

**Status: location decided 20 November 2026; download support implemented
behind the opt-in `fetch` cargo feature.** A Lens built with `fetch` can
download a pinned bundle with `munarium-lens fetch` or at startup (see
[Downloading](#downloading)). Every other build, and a `fetch` build without
bundle settings, downloads nothing: the operator fetches a bundle by hand (see
[Fetching a bundle by hand](#fetching-a-bundle-by-hand)) and points
`LENS_EMBEDDING_ARTIFACTS` or `LENS_RERANKING_ARTIFACTS` at it.
There is nothing to pin until the maintainer publishes a bundle and records
its commit and manifest digest (see [Publishing a bundle](#publishing-a-bundle)).

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

### Downloading

A Lens built with the `fetch` cargo feature downloads a pinned bundle in one
of two explicit ways. A build without the feature has no HTTP client or TLS
stack at all.

- **Off by default.** With no bundle settings, Lens behaves exactly as before:
  it loads only from the operator-supplied artifact directory and makes no
  outbound connection.
- **Two fixed locations, in a fixed order.** Lens downloads only from
  `https://huggingface.co/iokaio/munarium-lens-models` and, if that fails, from
  `https://github.com/iokaio/munarium-lens/releases/download/`. Both are
  compile-time constants. There is no setting or option for another base URL,
  a mirror, a different order or skipping Hugging Face, and Lens fetches from
  no other host for any reason. An egress proxy (below) changes only the
  route, never the hosts.
- **URLs.** Hugging Face:
  `https://huggingface.co/iokaio/munarium-lens-models/resolve/<commit>/<bundle>/<file>`.
  GitHub: `https://github.com/iokaio/munarium-lens/releases/download/models/<bundle>/<file>`.
  The tag `models/<bundle>` keeps its `/` unencoded, as GitHub's own
  `browser_download_url` does for such tags. Release assets are flat, so a
  bundle file in a subdirectory can come only from Hugging Face.
- **Manifest first.** Lens downloads `lens-manifest.json` (at most 64 KiB),
  requires the pinned SHA-256, checks it with the same code that loads a stage
  (schema, safe relative paths, digests) and requires its `served_model` to
  equal the bundle name. Only then does it download the files the manifest
  lists, and nothing else. If Hugging Face serves a different manifest, the
  GitHub copy is tried; if neither matches, no file is downloaded.
- **Redirects.** Followed manually, at most 5 per file, and only over HTTPS on
  port 443 to the hosts of the location being used: `huggingface.co` (which
  redirects small files to its own `/api/resolve-cache/` path),
  `cas-bridge.xethub.hf.co`, `cdn-lfs.hf.co`, `cdn-lfs-us-1.hf.co` and
  `cdn-lfs-eu-1.hf.co` for Hugging Face; `github.com`,
  `release-assets.githubusercontent.com` and `objects.githubusercontent.com`
  for GitHub. Any other target, user information in a URL or a downgrade to
  HTTP is refused before connecting.
- **Safe writes.** Each file streams into a `.lens-partial-*` temporary file in
  its target directory, is cut off at the manifest's size, and must match the
  size and SHA-256 exactly. It is then flushed to disk and renamed into place
  atomically. A failed download removes its temporary file and never leaves
  anything under the final name; the next source is then tried. Temporaries
  left by an interrupted process are removed on the next fetch. Files already
  present and verified are not downloaded again, and the manifest is written
  last. Finally the whole directory is verified again from disk.
- **Time bounds.** 30 s to resolve and connect, 60 s for response headers,
  60 s for any single read, and a total per file of
  `LENS_MODEL_FETCH_TIMEOUT_SECS` (default 3600) including redirects.
  Interrupted downloads restart from the beginning; there is no resume.
- **Egress proxy.** Lens honors the standard proxy settings through an HTTP
  `CONNECT` tunnel, with TLS end to end to the download host inside it, so
  certificate validation and the redirect allowlist apply to the target hosts
  exactly as on a direct connection. The proxy is a transport route only: it
  cannot change which hosts are fetched, its own answers are never followed
  as redirects, and a target that is not HTTPS is never sent through it.
  - `HTTPS_PROXY` or `https_proxy`: `http://[user:password@]host[:port]`
    (port 80 by default; a bare `host:port` means the same). Percent-encode
    reserved characters in the credentials. `https://` (TLS to the proxy) and
    SOCKS URLs are refused at startup with the variable named, not its value.
    If both spellings are set they must agree.
  - `NO_PROXY` or `no_proxy`: comma-separated host names, domains (matching
    their subdomains; a leading `.` or `*.` is optional), IP addresses, or `*`
    for all. CIDR ranges and ports are not supported and never match, so the
    proxy is used.
  - `ALL_PROXY` and `HTTP_PROXY` are deliberately not read: every download is
    HTTPS, and `ALL_PROXY` often names a SOCKS proxy, which is not supported.
  - The variables are read only by the `fetch` subcommand and by a server
    with a stage bundle; without a bundle they are ignored.
  - Proxy credentials are sent only to the proxy, in `Proxy-Authorization`,
    and are never logged; a proxy that refuses the tunnel logs `proxy_failed`.
- **No credentials.** Both locations are public. Lens sends them no token and
  accepts no extra headers. Failures log one fixed category per attempt, for
  example `Lens fetch: file from huggingface failed: file_digest_mismatch`,
  with no URL, path or value.

#### The `fetch` subcommand

```sh
munarium-lens fetch --bundle qwen3-embedding-0.6b-f32-1024d \
  --revision <40-char-commit> --manifest-sha256 <digest> \
  --cache-dir /var/cache/lens [--timeout-secs 3600]
```

It writes `<cache-dir>/<bundle>/`, prints the verified manifest digest on
standard output and exits 0. A failed fetch prints its category and exits 1; a
usage error exits 2. The revision must be a full 40-character lowercase commit
id: `main`, other branch names, tags and short ids are refused. There is no
option for a source, URL or order. The cache directory must not be inside a
Munarium Lens source checkout.

#### Fetching at startup

| Variable | Meaning |
|---|---|
| `LENS_EMBEDDING_BUNDLE`, `LENS_RERANKING_BUNDLE` | Bundle name (the manifest's `served_model`). Only for a `qwen3` stage, and only in a build with `fetch`; other builds refuse the setting. |
| `LENS_EMBEDDING_BUNDLE_REVISION`, `LENS_RERANKING_BUNDLE_REVISION` | Pinned Hugging Face commit, 40 lowercase hex characters. Required with the bundle. |
| `LENS_EMBEDDING_MANIFEST_SHA256`, `LENS_RERANKING_MANIFEST_SHA256` | The existing manifest digest pin. Required. |
| `LENS_MODEL_CACHE_DIR` | Writable directory for all bundles; each goes to `<dir>/<bundle>`. Required with a bundle and refused without one. Must not be inside a Munarium Lens source checkout. |
| `LENS_MODEL_FETCH_TIMEOUT_SECS` | Total time allowed per file, 1 to 86400 seconds. Default 3600. |

With a bundle set, the stage's artifact directory is `<cache>/<bundle>`, so
`LENS_*_ARTIFACTS` must stay unset. At startup Lens first verifies the cache
copy exactly as it would an operator-supplied directory. A complete, verified
copy loads with no connection at all. If the manifest or a listed file is
missing or does not match, Lens downloads what is missing or wrong, then
verifies and loads the directory as usual. This happens while the stages load,
before Lens answers any request; it never happens during request handling. If
the fetch or the verification fails, the stage stays `failed`, readiness
returns 503 and the log names one category. Lens does not retry until it is
restarted: retry is restart-only, and orchestrators should give the first
start a long startup probe (maintainer-approved, 20 November 2026).

In a container, use the `qwen3,fetch` image variant (see the
[local walkthrough](docs/guides/local-development.md#model-container)) with a
writable volume, such as a Kubernetes `emptyDir` or a persistent volume claim,
mounted at the cache directory, and a read-only root filesystem. A persistent
volume keeps the bundle across restarts, so a restart makes no connection.

- **Offline and air-gapped hosts.** Run `munarium-lens fetch` on a connected
  host, or fetch by hand, copy the bundle directory, and point
  `LENS_*_ARTIFACTS` at it read-only. The pinned manifest digest proves the copy
  is intact.
- **Dependency policy.** The fetcher uses `ureq` 3.5.2 with `rustls` 0.23 (the
  `ring` provider) and the compiled-in `webpki-roots` trust store: no OpenSSL,
  no platform TLS. [`deny.toml`](deny.toml) allows exactly these crates, only
  through their expected parents, with a scoped CDLA-Permissive-2.0 license
  exception for `webpki-roots`; the maintainer approved these exceptions on
  20 November 2026 for these two locations only. The default reference build
  and image contain none of them. The `qwen3` graph's one advisory exception,
  RUSTSEC-2024-0436 (`paste`, an unmaintained compile-time macro reached
  through Candle's `gemm`), was also approved on 20 November 2026; its
  reasoning and re-evaluation conditions are in `deny.toml`.

### Fetching a bundle by hand

Without the `fetch` feature, or to inspect a bundle first, use either
location directly, then pin the digest that `lens-manifest.json` produces:

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
7. After publishing, confirm the observed redirect hosts and trim the
   allowlist: fetch the new bundle from each location, note the hosts each
   download actually redirects to, and remove any host in `src/fetch.rs`
   (and its test) that neither location used (maintainer-approved,
   20 November 2026).

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
