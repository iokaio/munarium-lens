# Artifacts and offline serving

Each stage has an immutable manifest in an operator-managed, read-only mount.
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

1. Run the importer under explicit operator authorization, from local assets or
   a separately authorized verified fetch. Check license and pins, constrain
   paths under a staging directory, reject unexpected files, symlinks and
   path traversal, verify size and SHA-256, then atomically publish a read-only
   non-overwriting installation on the same filesystem.
2. Serve with the imported mount, outbound network denied and model downloads
   disabled. Validate the configured manifest and all referenced bytes before
   loading. Keep the mount read-only for the life of the process to prevent
   drift between check and use. Load only the permitted format using the
   approved runtime configuration, with remote or artifact-supplied code disabled.
3. Run the synthetic probe against the loaded instance. Report ready only when
   load, identity and response validation all pass. Refuse inference from a bad
   or warming profile, and remove unhealthy workers from the corresponding
   pool. Keep import and startup diagnostics limited to approved identifiers
   and fixed error categories.

Terminate authenticated transport at the service or a trusted, scoped ingress
with protected forwarding. Choose the deployment credential scheme with IR-1,
verify rotation and revocation, and restrict network access to approved Server
replicas/builders and control probes. Never emit credentials, input text,
returned embeddings or passages in logs. [Serving guide](../ops/serving.md)
