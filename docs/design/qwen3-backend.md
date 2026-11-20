# Qwen3 backend on Candle

**Status: implemented behind the opt-in `qwen3` cargo feature; not
qualified.** This page describes how Lens serves the accepted pair,
Qwen3-Embedding-0.6B and Qwen3-Reranker-0.6B, on the accepted Candle runtime.
See the [model runtime page](model-runtime.md) and the [decision register](decisions.md).
The default build and the released image remain reference-only. LENS-2
qualification, service authentication and the importer are still pending.

## Build

```sh
cargo build --locked --release --features qwen3
```

The feature adds `candle-core`, `candle-nn` and `candle-transformers` 0.11.1,
`tokenizers` 0.23.2 (pure-Rust `fancy-regex`, without the default Oniguruma
and C++ suffix-array features), `sha2` 0.11.1 and `serde_json` 1.0.154 as a
normal dependency. For the Linux musl target the feature graph has 174
crates against 44 for the default build. Each crate offers a license
allowed by [`deny.toml`](../../deny.toml), mostly MIT or Apache-2.0, and the
advisory supply-chain workflow runs `cargo deny check` on both graphs. The
only advisory is RUSTSEC-2024-0436: `paste` is unmaintained and arrives as a
compile-time proc-macro through Candle's `gemm`. `deny.toml` records the
reasoning, pending maintainer acceptance. None of the crates compiles C or
C++ code or downloads anything at build or run time. Candle runs on CPU in FP32; its
CUDA and Metal features stay off.

## Configuration

| Variable | Meaning |
|---|---|
| `LENS_EMBEDDING_BACKEND`, `LENS_RERANKING_BACKEND` | `qwen3` selects this backend for the stage. A build without the feature refuses the value. |
| `LENS_EMBEDDING_ARTIFACTS`, `LENS_RERANKING_ARTIFACTS` | Operator-supplied directory holding `lens-manifest.json` and the files it lists. Mount it read-only. Required for a `qwen3` stage. |
| `LENS_EMBEDDING_MANIFEST_SHA256`, `LENS_RERANKING_MANIFEST_SHA256` | Lowercase hex SHA-256 of the manifest file's exact bytes. Required with the directory. |

`LENS_MAX_INPUT_TOKENS` must not exceed the model's `max_position_embeddings`,
or the stage does not load.

## Artifact manifest and verification

The manifest (`munarium-lens-manifest/0`, proposed) is JSON with no unknown
fields:

```json
{"format":"munarium-lens-manifest/0","stage":"embedding","backend":"qwen3",
 "served_model":"qwen3-embedding-0.6b-f32-1024d","precision":"f32","dimensions":1024,
 "upstream":{"repository":"Qwen/Qwen3-Embedding-0.6B","revision":"97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3"},
 "files":[{"path":"config.json","size":727,"sha256":"…"},
          {"path":"tokenizer.json","size":11423705,"sha256":"def76fb0…"},
          {"path":"model.safetensors","size":1191586416,"sha256":"0437e45c…"}]}
```

A reranking manifest uses `"stage":"reranking"` and omits `dimensions`.
`python scripts/lens_manifest.py` writes one for a local directory and prints
its digest. At startup, before anything loads, Lens:

1. reads `lens-manifest.json` (a regular file of at most 64 KiB, not a
   symlink) and requires its SHA-256 to equal the configured digest;
2. checks the format, the stage, `backend: qwen3`, `precision: f32`, the
   dimension, a served name of at most 128 characters from `[A-Za-z0-9._/-]`,
   and that `config.json`, `tokenizer.json` and `model.safetensors` are listed;
3. rejects absolute, parent-relative, backslash, drive-letter and repeated
   paths, and more than 64 files;
4. reads every listed file once, refusing symlinks and non-regular files and
   reading no more than the declared size, and checks the size and SHA-256;
5. loads the configuration, tokenizer and weights from the bytes it verified,
   so a file changed after the check is never read; and
6. runs the existing content-free startup probe.

Any failure leaves the stage `failed`, so readiness returns 503, and logs one
fixed category such as `manifest_digest_mismatch` or `file_digest_mismatch`
without paths or values. Readiness reports `backend: "qwen3"`,
`retrieval_model: true`, the manifest's served name and `manifest_sha256`.

The digest over the manifest's exact bytes is provisional: the model artifact
owner still freezes a canonical serialization and the full manifest of the
[artifact design](artifacts-and-serving.md). The served name comes from the
manifest, so the operator who pins its digest also pins the name; it must
change with the weights, tokenizer, precision or template.

## Embedding, following the model card

The Qwen3-Embedding card's Transformers example tokenizes with the
publisher's tokenizer, takes the final hidden state of the last token and L2
normalizes it. Lens does the same:

- `tokenizer.json`'s post-processor appends `<|endoftext|>` to every input,
  as the card relies on. Lens encodes with special tokens enabled and counts
  those tokens, so an empty string is one token.
- The model runs one input at a time, so the last position is the last real
  token (the card's left-padding case). Candle's Qwen3 model has no padding
  mask, so Lens never pads or batches across inputs.
- The vector is the final-norm hidden state at that position, L2 normalized,
  at the native 1,024 dimensions. Nothing is truncated.
- Lens adds no instruction. Server sends queries already prefixed, for
  example `Instruct: {task}\nQuery:{query}` as on the card, and documents
  without a prefix.

The publisher's embedding weights omit the `model.` tensor prefix that
Candle's Qwen3 loader expects; Lens maps the names when that layout is found.

## Reranking, following the model card

The Qwen3-Reranker card builds each pair as a chat prompt and reads the
logits of `yes` and `no` at the final position:

```text
<|im_start|>system
Judge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be "yes" or "no".<|im_end|>
<|im_start|>user
<Instruct>: {instruction}
<Query>: {query}
<Document>: {document}<|im_end|>
<|im_start|>assistant
<think>

</think>

```

As the card's Transformers example does, Lens tokenizes the fixed prefix and
suffix without special tokens and the pair content on its own, then joins
the three. The reranker's tokenizer adds no special tokens. The `yes` and
`no` ids come from the tokenizer's vocabulary. The score is
`logit(yes) − logit(no)`, which is the card's Sentence Transformers default
and the log-odds of the card's Transformers probability:
`P(yes) = sigmoid(score)`. The order is the same. Lens returns the log-odds
because a probability near 1.0 rounds to equal `f32` values for strongly
relevant candidates, which would turn real differences into ties.

### Template and instruction split

| Part | Owner | Why |
|---|---|---|
| System prompt, the `im_start` and `im_end` role markers, empty `<think>` block | Lens, fixed model template | The card fixes them for every task. They are part of the served identity and counted in `usage`. |
| `\n<Document>: ` between the two wire fields | Lens, fixed model template | It joins `query` and each `documents` item, which arrive separately. |
| `<Instruct>: {instruction}\n<Query>: ` before the query | **Server**, as a query prefix | The instruction is task-specific and Server owns instructions. The labels go with it, just as Server's embedding prefix carries `Instruct:` and `Query:`. |

With Server's prefix, Lens's prompt equals the card's byte for byte; a unit
test checks this. Lens never adds instruction text. If Server sends a bare
query, the prompt lacks the `<Instruct>` and `<Query>` lines, a form the card
does not document. Server's binding has no rerank instruction setting yet;
see the open questions below.

## Observed on a development machine

Indicative only, not a LENS-2 receipt: runs on 20 November 2026 on a
Windows laptop (Intel Core i7-10750H, 6 cores and 12 threads, 64 GB RAM),
release build, FP32, CPU, default Candle threading, other work idle. The
weights are the pinned revisions in the [model runtime page](model-runtime.md),
whose SHA-256 values matched the publisher's LFS object ids.

| Check | Result |
|---|---|
| Embedding parity with the card's Transformers example (cosine of 2 queries × 2 documents) | 0.7645581, 0.1414258, 0.1354979, 0.5999566 against the card's 0.7645568, 0.1414251, 0.1354974, 0.5999550: within 2×10⁻⁶ |
| Reranking parity with the card's Sentence Transformers example (logit difference) | 7.597 and −12.192 against the card's 7.625 and −11.375, which are multiples of 1/8 and so appear to come from reduced precision. As the card's probability `sigmoid(score)`: 0.99950 against 0.99951, and 5.1×10⁻⁶ against 1.15×10⁻⁵. Same order. A Candle BF16 run to confirm the cause fails on the CPU path, so the difference is recorded, not explained. |
| Load and verify both stages (hash 2.4 GB, convert BF16 to FP32) | 74 s in the test, 52 s to readiness in the service |
| Embed the card's 4 short inputs | 5.1 s |
| Rerank the card's 2 pairs (the first is 85 tokens) | 8.4 s |
| Rerank 1 pair of 639 tokens | 20.1 s |
| Peak working set with both stages loaded | 5.8 GiB |

These latencies are far from Server's query deadline for 20 to 50
candidates: one sequence at a time on a laptop CPU, with no batching. The
earlier AIRK measurement already marked a 0.6B reranker at 50 candidates as
a GPU workload. LENS-2 measures the approved hardware.

Run it with the publisher's files in an ignored directory such as `models/`:

```sh
LENS_QWEN3_TEST_DIR=models cargo test --release --features qwen3 -- --ignored --nocapture
```

## Container image

The [Dockerfile](../../Dockerfile) builds a `qwen3` variant with
`--build-arg LENS_FEATURES=qwen3`; the default build argument is empty and
produces the reference image. The variant uses the same static musl builder
and distroless runtime, because the feature graph is pure Rust. No glibc
base or native library is needed. Weights stay outside the image:

```sh
docker build --build-arg LENS_FEATURES=qwen3 -t munarium-lens-qwen3:local .
docker run --rm -p 127.0.0.1:8787:8787 -v /srv/lens:/models:ro \
  -e LENS_HOST=0.0.0.0 -e LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK=1 \
  -e LENS_EMBEDDING_BACKEND=qwen3 -e LENS_EMBEDDING_ARTIFACTS=/models/qwen3-embedding-0.6b -e LENS_EMBEDDING_MANIFEST_SHA256=<digest> \
  -e LENS_RERANKING_BACKEND=qwen3 -e LENS_RERANKING_ARTIFACTS=/models/qwen3-reranker-0.6b -e LENS_RERANKING_MANIFEST_SHA256=<digest> \
  munarium-lens-qwen3:local
```

Indicative local result on the same laptop under Docker Desktop: the image
is 12.9 MB, both stages became ready in 105 s from a Windows bind mount,
a two-pair rerank of 170 tokens in total took 4.2 to 4.8 s, and the container used
4.5 GiB. Musl's allocator and the absence of a GPU are LENS-2 questions; a
GPU build would need a glibc base with CUDA libraries and is not provided.
No variant image is published.

## Open questions

- **Rerank instruction in Server.** Server's binding has an embedding
  `query_instruction` but no rerank instruction, and it sends the rerank
  query unchanged. To match the card, Server would prefix
  `<Instruct>: {instruction}\n<Query>: `. The alternative is an explicit
  instruction field on the wire, with the labels in the Lens template. Either
  needs a hub decision before Server's consumer changes.
- **Query prefix spacing.** The embedding card's prompt is
  `Instruct: {task}\nQuery:{query}` with no space after `Query:`. Server's
  documented example ends in `Query: `. Both are Server's choice and part of
  its semantic space; LENS-3 can compare them.
- **Throughput.** One sequence at a time is correct but slow (see the
  measurements above). Padded batching needs an attention mask that
  Candle's Qwen3 model does not take; a Lens-owned model variant or a GPU
  build is a LENS-2 decision.
- **Manifest canonical form and importer.** See the artifact design.
