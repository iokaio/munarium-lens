# Artifacts and offline serving

Each stage has an immutable manifest in an operator-managed, read-only mount,
or, with the opt-in [verified fetch](#verified-fetch-implemented-opt-in), in a
writable cache that Lens fills from the two fixed distribution locations.
The shipping image pins its own base-image digest and runtime dependencies.
Keep weights outside the image. Record their separate redistribution decision
alongside every dependency and fixture in the release notices.

| Manifest section | Required fields |
|---|---|
| Schema and identity | Format version, stage, immutable served model name, canonical manifest digest method and expected digest. |
| Upstream | Model and tokenizer repositories and exact revisions, format, attribution, license and redistribution decision for every artifact. |
| Files | Relative paths, byte sizes and SHA-256 per file. Include tokenizer, templates, configuration and all weight parts. |
| Execution | Runtime version and build, precision and quantization parameters, device class, embedding dimensions/truncation and normalization, query and passage instructions, rerank template and tie policy. |
| Checks | Content-free cold-start probe revision, length and batch limits, expected shape and applicable numerical tolerance. Record image digest and qualification receipt references alongside the verified profile. |

The model artifact owner freezes canonical serialization and digest vectors
before exporting the manifest format. A change to weights, tokenizer, precision,
instructions or vector interpretation gets a distinct identity. Key caches,
readiness, enrollment and routing by the full approved binding. A new embedding
identity requires a new compatible index.

1. Run the [offline importer](../../README.md#importing-a-downloaded-bundle-offline)
   on local assets or use the verified fetch. Check license and pins. The
   importer stages only listed regular files, verifies size and SHA-256, and
   publishes a complete installation on the same filesystem. Give the
   serving identity read access and mount the installation read-only.
2. Serve with the imported mount, outbound network denied and model downloads
   disabled, or with the opt-in startup fetch described below. Validate the configured manifest and all referenced bytes before
   loading. Keep the mount read-only for the life of the process to prevent
   drift between check and use. Load only the permitted format using the
   approved runtime configuration, with remote or artifact-supplied code disabled.
3. Run the synthetic probe against the loaded instance. Report ready only when
   load, identity and response validation all pass. Refuse inference from a bad
   or warming profile, and remove unhealthy workers from the corresponding
   pool. Keep import and startup diagnostics limited to approved identifiers
   and fixed error categories.

## Verified fetch (implemented, opt-in)

The maintainer decided on 20 November 2026 that bundles are published at
`huggingface.co/iokaio/munarium-lens-models`, with a GitHub Releases backup in
this repository under `models/<bundle>` tags, and that Lens may download
them, because baking weights into the image would make it far too large.
The `fetch` cargo feature implements that verified fetch. It is the only
outbound network use in Lens, and only these two locations are allowed:

- **Fixed sources and order.** Both base URLs are compile-time constants, tried
  Hugging Face first and the GitHub backup only if Hugging Face fails. No
  setting or option names a source, URL, mirror or order; only a `cfg(test)`
  build can substitute a local test server, and a test fails if any
  non-test source module reads the environment or configuration reads an
  unreviewed variable. Redirects stay on HTTPS to each source's documented
  download hosts. A standard `HTTPS_PROXY`/`NO_PROXY` egress proxy is a
  transport route only: an HTTP `CONNECT` tunnel with TLS end to end to the
  same hosts, chosen per request after the allowlist check.
- **Pins.** Bundle name, the full Hugging Face commit (branches and tags are
  refused) and the manifest SHA-256. The manifest is downloaded first and
  checked with the same parse and path checks as an operator directory; only
  listed files are then downloaded.
- **Staging and publication.** Each file streams into a temporary file beside
  its final name, capped at the listed size, and is renamed into place only
  after its size and SHA-256 match, so a partial or wrong file never exists
  under a listed name. The completed directory is verified again from disk,
  then stage loading verifies it in full as in step 2.
- **When.** Through `munarium-lens fetch`, or at startup for a stage with
  `LENS_*_BUNDLE` settings, before Lens answers requests; never during request
  handling. A stage whose bundle cannot be fetched and verified stays
  `failed`. Without bundle settings Lens makes no outbound connection.

This fetch is not the importer above: it does not check license decisions or
reject unlisted files already present in the cache directory (it neither
reads nor loads them), and it writes into a writable cache rather than a
read-only installation. Deployments that need the importer's guarantees keep
using a separately imported, read-only mount. Details, settings and failure
categories are in the [README](../../README.md#downloading).

Terminate authenticated transport at the service or a trusted, scoped ingress
with protected forwarding. Choose the deployment credential scheme with IR-1,
verify rotation and revocation, and restrict network access to approved Server
replicas/builders and control probes. Never emit credentials, input text,
returned embeddings or passages in logs. [Serving guide](../ops/serving.md)
