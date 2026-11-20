# Model runtime and first model pair

**Status: model pair and runtime accepted.** On 20 November 2026 the
maintainer accepted pair **A**, Qwen3-Embedding-0.6B with Qwen3-Reranker-0.6B,
at 1,024 dimensions, and assigned instruction prefixes to Server
([Server record](https://github.com/iokaio/munarium/blob/89e1bc2019c80d697d2d8adc9237a9ce577cf3d7/server/docs/design/implementation-ledger.md#maintainer-decisions-20-november-2026)).
The same day the maintainer confirmed Candle as the runtime, noting "we can
alter that later if needed". The [decision register](decisions.md) tracks
each item. Precision, supported hardware, serving limits and quality
thresholds stay open until LENS-2 measures them, as listed under
[what qualification measures](#what-lens-2-and-lens-3-measure).

## Decisions of 20 November 2026

| Item | Decision | Consequence in Lens |
|---|---|---|
| Model pair | **Accepted:** Qwen3-Embedding-0.6B and Qwen3-Reranker-0.6B at the revisions pinned below | Pairs B, C and D remain in the comparison below as a record. D can still serve as a CPU smoke test; it is not a release candidate. |
| Dimension | **Accepted:** 1,024, the embedder's native output | No truncation. Matches Server's proposed storage trial. |
| Instruction prefixes | **Accepted:** Server owns them; Lens applies none | Consistent with the [endpoint profile](endpoint-profile.md#lens-1-implemented-profile-clarifications-proposed). Each embedding input and rerank query arrives as the effective text. Lens owns only the model's fixed template, such as the reranker's chat frame. The identity still records both. |
| Runtime | **Accepted:** Candle (`candle-core`, `candle-nn` and `candle-transformers` 0.11.1), first on CPU behind an opt-in `qwen3` cargo feature. The maintainer noted the choice can be altered later if needed. | The default build stays the reference-only build without ML dependencies. Candle loads publisher safetensors directly and builds statically. If Candle misses the LENS-2 latency gate, a GPU build, ONNX Runtime or a sidecar can be proposed as a change to this decision. |

The [Qwen3 backend page](qwen3-backend.md) describes the implementation
behind the `qwen3` feature, its manifest verification and indicative
measurements. Until a model-backed profile is qualified,
the release image serves only the deterministic reference backend, which is
not a retrieval model. Every backend plugs into the per-stage interfaces in
`src/backend.rs` without a wire change: the backend reports its served
identity, counts tokens with its pinned tokenizer and template, and returns
vectors or scores that the server validates before replying.

## Fixed constraints

These follow from the existing designs and narrow the choice.

- Weights stay outside the image and load from a verified, read-only mount.
  Serving runs with downloads and outbound network disabled. (Since the
  maintainer decision of 20 November 2026, the opt-in `fetch` feature may
  download pinned, verified bundles before serving, only from the two fixed
  Munarium locations; see the [README](../../README.md#downloading). The
  runtime itself still downloads nothing.) Build-time or
  run-time binary downloads by a crate (for example a runtime's
  download-binaries feature) are excluded; native libraries come from a
  pinned base image or a pinned, checksummed build input.
- No artifact-supplied or remote code. A model that needs custom modelling
  code at load time is excluded unless that code is reimplemented, reviewed
  and pinned in Lens.
- One adapter per stage, independently routed pools, and a distinct served
  name per weight, tokenizer, precision, template and dimension revision.
  Server's semantic space records the instructions it applies.
- Server proposes a 1,024-dimension first trial alongside its 256-dimension
  hash index, within pgvector's 2,000-dimension HNSW limit. A smaller
  dimension needs its own Server storage path or truncation recorded in the
  identity. [Server storage proposal](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/airk-analysis.md#85-what-the-feature-does-in-server)
- Reranking must score the Server's default 50 fused candidates within the
  query deadline. AIRK measured about 2.1 s p95 for four 512-token pairs with
  an 8-bit 0.6B reranker on four CPU threads, so a 0.6B reranker at 50
  candidates is a GPU workload, and a CPU-only profile needs a much smaller
  reranker or a lower candidate cap. [AIRK measurements](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/airk-analysis.md#83-what-airk-has-measured-and-what-it-has-not)

## Model pair candidates

This comparison informed the decision above and is kept as a record. Facts were read from each publisher's Hugging Face model API and `config.json`
at the pinned revision on 20 November 2026. The revision links are stable.
Re-read the license file and model card at freeze; a card's license tag is
not a rights review. Sizes are the publisher's single safetensors file.

| Pair | License (pinned source) | Parameters, dimension, max positions | Weight size at publisher precision | Runtime options | Notes |
|---|---|---|---|---|---|
| **A. Qwen3-Embedding-0.6B + Qwen3-Reranker-0.6B** (accepted 20 November 2026) | Apache-2.0 · [embedding @97b0c61](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B/tree/97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3) · [reranker @e61197e](https://huggingface.co/Qwen/Qwen3-Reranker-0.6B/tree/e61197ed45024b0ed8a2d74b80b4d909f1255473) | 596M each, decoder-only `qwen3`; 1,024 native, publisher-documented smaller output dimensions; 32,768 | 1.19 GB each, BF16 | No publisher ONNX export. vLLM and llama.cpp list support for both (verify on the pinned build); Candle has a Qwen3 model (verify embedding pooling). ONNX needs a Lens-owned, reviewed export. | Matches the 1,024 trial. Server applies the query instruction; the reranker's yes/no chat template is part of the Lens identity. The reranker reads two logits per pair, so each candidate is a full forward pass. AIRK's code-search AUROC with the default web-search instruction was near chance, which LENS-3 must address. |
| **B. bge-m3 + bge-reranker-v2-m3** | MIT · [bge-m3 @5617a9f](https://huggingface.co/BAAI/bge-m3/tree/5617a9f61b028005a4858fdac845db406aefb181); Apache-2.0 · [reranker @953dc6f](https://huggingface.co/BAAI/bge-reranker-v2-m3/tree/953dc6f6f85a1b2dbfca4c34a2796e7dde08d41e) | about 568M each, encoder `xlm-roberta`; 1,024; 8,194 | 2.27 GB each, FP32 (bge-m3 ships `pytorch_model.bin` plus a publisher ONNX export) | Publisher ONNX for the embedder (external-data file); reranker needs an export. ORT, Candle (XLM-R) and TEI are plausible. | Mixed licenses within the pair. Encoder-only, no instruction prefix by default. Proposed: exclude pickle-based PyTorch files as a permitted load format, so the embedder would use its ONNX export or a reviewed conversion. |
| **C. gte-modernbert-base + gte-reranker-modernbert-base** | Apache-2.0 · [embedding @e7f32e3](https://huggingface.co/Alibaba-NLP/gte-modernbert-base/tree/e7f32e3c00f91d699e8c43b53106206bcc72bb22) · [reranker @f7481e6](https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base/tree/f7481e6055501a30fb19d090657df9ec1f79ab2c) | 149M each, encoder `modernbert`; 768; 8,192 | 0.30 GB FP16 and 0.60 GB FP32 | Publisher ONNX exports in FP32, FP16, INT8 and 4-bit. ORT is direct; tract and Candle support for ModernBERT needs checking. | Smallest pair with long context and a single license. A CPU-capable reranker at 50 candidates is plausible but unmeasured. 768 dimensions need Server storage beyond the proposed 1,024 trial. |
| **D. all-MiniLM-L6-v2 + ms-marco-MiniLM-L6-v2** (CPU floor) | Apache-2.0 card tags · [embedding @1110a24](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/tree/1110a243fdf4706b3f48f1d95db1a4f5529b4d41) · [reranker @233902d](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2/tree/233902d25c440f23af6f7d6e94d2946bac0bee0a) | 23M each, `bert`; 384; 512 | 0.09 GB each, FP32 | Publisher ONNX, including optimized and INT8 variants. Every listed runtime, including pure-Rust tract and rten. | Useful as a cheap CPU baseline and runtime smoke test, not as the release profile. Both were trained with MS MARCO data, whose terms need a separate rights review before redistribution or commercial use. 512 positions and 384 dimensions are below the Server trial. |

Excluded at screening: Snowflake arctic-embed-m-v2.0 (Apache-2.0) is tagged as
requiring custom model code, and models with non-commercial licenses are not
eligible for a public Apache-2.0 release profile.

## Runtime candidates

Crate versions and licenses were read from the crates.io index on
20 November 2026. "Image impact" is the shipped image only; weights are
mounted. All figures marked *estimate* need measurement in LENS-2.

| Runtime | License | CPU | GPU | Lens build impact | Image impact | Assessment |
|---|---|---|---|---|---|---|
| ONNX Runtime through the `ort` crate (2.0.0-rc.15) | ORT: MIT ([license](https://github.com/microsoft/onnxruntime/blob/main/LICENSE)); `ort`: MIT OR Apache-2.0 | Broad operator coverage, INT8 and FP16 | CUDA and other execution providers | C++ shared library linked against glibc; no static musl image. Disable binary download and link a pinned library. | CPU library tens of MB (*estimate*); CUDA execution provider plus CUDA and cuDNN libraries in the GB range (*estimate*). Base moves from distroless static to a glibc base. | Most model coverage (B, C, D directly; A after a reviewed export). A native crash takes the worker down, which the pool design tolerates but must be tested. |
| Candle (`candle-core`/`candle-transformers` 0.11.1) | MIT OR Apache-2.0 ([repository](https://github.com/huggingface/candle)) | Pure Rust, safetensors directly | CUDA and Metal features | Pure Rust on CPU, static musl possible. CUDA needs the toolkit at build and a glibc base. | CPU adds a few MB (*estimate*); CUDA as for ORT. | Loads publisher safetensors without conversion. Model-family coverage and numerical parity with the publisher reference must be shown per family. |
| tract (`tract-onnx` 0.23.10) | MIT OR Apache-2.0 ([repository](https://github.com/sonos/tract)) | Pure Rust ONNX and NNEF | None | Static musl, no native code | A few MB (*estimate*) | CPU-only. Good for D; support for ModernBERT, XLM-R at length and Qwen3 graphs is unverified. |
| rten (0.28.0) | MIT OR Apache-2.0 ([repository](https://github.com/robertknight/rten)) | Pure Rust, converted `.rten` models | None | Static musl; Server already uses it for OCR | A few MB (*estimate*) | CPU-only and needs a conversion step whose output becomes a pinned artifact. Server tracks a pure-Rust in-process experiment as IR-6. |
| Tokenization: `tokenizers` (1.0.0-rc.4) | Apache-2.0 ([repository](https://github.com/huggingface/tokenizers)) | Needed by every in-process option | — | Choose a pure-Rust regex feature to keep static builds | Small | Must reproduce the publisher's special tokens and templates; the count it returns drives Lens's token limits. |
| Sidecar server: vLLM or text-embeddings-inference behind Lens | Both Apache-2.0 per their repositories ([vLLM](https://github.com/vllm-project/vllm), [TEI](https://github.com/huggingface/text-embeddings-inference)) | TEI yes; vLLM is GPU-first | Yes | Lens becomes a governance proxy; the adapter makes HTTP calls instead of in-process work | Separate image of several GB (*estimate*) with its own notices | Fastest route to A on GPU and already the Server's bring-your-own model. Lens would still own identity, limits and refusals, but artifact verification and offline loading would depend on the sidecar's configuration. |
| llama.cpp (GGUF) | MIT ([repository](https://github.com/ggml-org/llama.cpp)) | Yes, quantized | CUDA and others | C++ through bindings or a sidecar | Moderate (*estimate*) | Documents Qwen3 embedding and reranking. GGUF conversion and quantization create new identities that need their own qualification. |

## Path after the 20 November 2026 decisions

Each item follows the accepted decisions.

1. Keep Lens in Rust and keep the reference backend for contract tests.
2. Run LENS-2 on pair **A** against the hash baseline. Pair C or D may serve
   as a comparison, but the release profile is pair A unless the maintainer
   revisits the decision on measured quality.
3. Implement pair A first on Candle, CPU only, in FP32 behind the `qwen3`
   feature. Record parity with the publisher's reference values and the
   measured latency. If Candle on the approved hardware misses the latency
   gate, compare an ONNX Runtime or GPU build, or a sidecar, and ask the
   maintainer to revisit the runtime decision.
4. Serve 1,024 dimensions with no truncation, matching Server's proposed
   storage, and record precision per stage (FP32 reference first, then FP16
   or BF16 on GPU and reduced precision on CPU only after drift and quality pass).

## What LENS-2 and LENS-3 measure

For every candidate profile (pair, runtime build, precision, dimension,
instructions, device and limits), record against frozen judgments:

- **Correctness against a reference.** Per-vector cosine to the publisher's
  FP32 reference implementation on fixed inputs, rerank order agreement and
  score drift with a predeclared tolerance; the cold-start probe values.
- **Retrieval quality per family (I3).** recall@k and nDCG@k on held-out
  documentation and source code separately, for hash only, embed only,
  rerank only and both, with two passing family receipts for one profile.
- **Latency and capacity.** Cold load time, warm p50/p95/p99 for embedding
  batches and for 50-candidate reranks within Server's deadline, throughput
  for index builds with interactive queries running, queue age, refusals.
- **Resources.** Peak host and device memory per stage, co-located and
  separate, image size and digest, mounted artifact size and digests.
- **Failure handling.** Missing or corrupt artifacts, overlong input,
  overload, timeout and worker removal, with content-free diagnostics.
- **Instructions (LENS-3).** Query, passage and rerank instructions compared
  with publisher defaults on held-out examples. Server applies each variant,
  so a variant is part of Server's semantic space rather than a new Lens
  served identity.
- **Rights.** License text, attribution and redistribution decision for every
  weight, tokenizer, dataset term and runtime library in the shipped graph.
