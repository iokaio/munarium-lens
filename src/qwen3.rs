// SPDX-License-Identifier: Apache-2.0

//! Qwen3-Embedding and Qwen3-Reranker backends on Candle, CPU, FP32.
//!
//! Built only with the `qwen3` cargo feature. Each stage loads from an
//! operator-supplied directory verified against its manifest (see
//! `manifest.rs`), so nothing is downloaded and only verified bytes load.
//!
//! **Embedding** follows the Qwen3-Embedding model card: the publisher's
//! tokenizer appends `<|endoftext|>`, the embedding is the final-layer
//! hidden state of the last token (last-token pooling) and the vector is L2
//! normalized. The dimension is the model's native 1,024; nothing is
//! truncated. Lens adds no instruction: Server sends the effective text,
//! including any `Instruct: ...\nQuery:` prefix.
//!
//! **Reranking** follows the Qwen3-Reranker model card. Each pair is the
//! card's chat frame around `{query}\n<Document>: {document}`, tokenized as
//! prefix, content and suffix the way the card does. The score is the
//! final-position logit of `yes` minus that of `no`. That is the Sentence
//! Transformers default of the card and the log-odds of the card's
//! Transformers probability, `P(yes) = sigmoid(score)`; the order is the
//! same and the log-odds does not saturate in `f32` the way a probability
//! near 1.0 does.
//!
//! Template versus instruction: the chat frame (system prompt, role markers,
//! the empty thinking block) and the `\n<Document>: ` separator are fixed
//! model template, owned by Lens and part of the served identity. The task
//! instruction is Server's. The card writes the user turn as
//! `<Instruct>: {instruction}\n<Query>: {query}\n<Document>: {document}`;
//! Server owns `<Instruct>: {instruction}\n<Query>: ` as a query prefix, in
//! the same way it owns `Instruct: ...\nQuery:` for the embedder. With that
//! prefix the token sequence equals the card's exactly.
//!
//! Each sequence runs alone (batch of one), because Candle's Qwen3 model has
//! no padding mask; inputs are never padded or truncated.

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3::{Config as ModelConfig, Model, ModelForCausalLM};
use tokenizers::Tokenizer;

use crate::backend::{BackendError, EmbeddingBackend, RerankingBackend, ServedIdentity};
use crate::manifest::{self, ArtifactError, Expected, Verified};

/// Backend family label reported by readiness and discovery.
pub(crate) const BACKEND: &str = "qwen3";
/// The precision this build computes in. The manifest must declare it.
pub(crate) const PRECISION: &str = "f32";
/// The embedder's native dimension, accepted on 20 November 2026.
pub(crate) const DIMENSIONS: usize = 1024;
/// Files a stage loads. The manifest may list more; those are verified only.
pub(crate) const REQUIRED_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

/// Fixed reranker chat frame before the pair content (model card).
pub(crate) const RERANK_PREFIX: &str = "<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be \"yes\" or \"no\".<|im_end|>\n<|im_start|>user\n";
/// Fixed reranker chat frame after the pair content (model card).
pub(crate) const RERANK_SUFFIX: &str = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n";
/// Separator between the query side and the document (model card).
pub(crate) const DOCUMENT_SEPARATOR: &str = "\n<Document>: ";

/// The pair content between the fixed prefix and suffix.
pub(crate) fn rerank_content(query: &str, document: &str) -> String {
    format!("{query}{DOCUMENT_SEPARATOR}{document}")
}

/// Final-layer hidden state of the last position of a `(1, len, hidden)`
/// tensor. The tokenizer ends every input with `<|endoftext|>`, so this is
/// the model card's last-token pooling for an unpadded sequence.
pub(crate) fn last_token(hidden: &Tensor) -> candle_core::Result<Vec<f32>> {
    let (_, len, _) = hidden.dims3()?;
    hidden
        .narrow(1, len - 1, 1)?
        .flatten_all()?
        .to_vec1::<f32>()
}

/// L2-normalize a vector. `None` for a zero or non-finite norm.
pub(crate) fn l2_normalize(mut vector: Vec<f32>) -> Option<Vec<f32>> {
    let norm = vector
        .iter()
        .map(|&value| f64::from(value) * f64::from(value))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return None;
    }
    for value in &mut vector {
        *value = (f64::from(*value) / norm) as f32;
    }
    Some(vector)
}

/// Relevance score from final-position logits: `logit(yes) - logit(no)`.
pub(crate) fn yes_no_score(logits: &[f32], yes: u32, no: u32) -> Option<f32> {
    let yes = *logits.get(yes as usize)?;
    let no = *logits.get(no as usize)?;
    Some(yes - no)
}

fn parse_config(bytes: &[u8]) -> Result<ModelConfig, ArtifactError> {
    serde_json::from_slice(bytes).map_err(|_| ArtifactError::LoadFailed)
}

fn parse_tokenizer(bytes: &[u8]) -> Result<Tokenizer, ArtifactError> {
    Tokenizer::from_bytes(bytes).map_err(|_| ArtifactError::LoadFailed)
}

/// Build a `VarBuilder` over verified safetensors bytes. Accepts both the
/// `model.`-prefixed causal-LM layout (reranker) and the bare layout the
/// embedding repository uses.
fn var_builder(weights: Vec<u8>) -> Result<VarBuilder<'static>, ArtifactError> {
    let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &Device::Cpu)
        .map_err(|_| ArtifactError::LoadFailed)?;
    if vb.contains_tensor("model.embed_tokens.weight") {
        Ok(vb)
    } else if vb.contains_tensor("embed_tokens.weight") {
        Ok(vb.rename_f(|name| name.strip_prefix("model.").unwrap_or(name).to_owned()))
    } else {
        Err(ArtifactError::LoadFailed)
    }
}

fn token_ids(tokenizer: &Tokenizer, text: &str) -> Result<Vec<u32>, BackendError> {
    tokenizer
        .encode(text, true)
        .map(|encoding| encoding.get_ids().to_vec())
        .map_err(|_| BackendError::Unavailable)
}

fn ids_tensor(ids: &[u32]) -> Result<Tensor, BackendError> {
    Tensor::new(ids, &Device::Cpu)
        .and_then(|tensor| tensor.unsqueeze(0))
        .map_err(|_| BackendError::Unavailable)
}

fn check_positions(config: &ModelConfig, max_input_tokens: usize) -> Result<(), ArtifactError> {
    if max_input_tokens > config.max_position_embeddings {
        return Err(ArtifactError::ManifestProfileMismatch);
    }
    Ok(())
}

pub(crate) struct Qwen3Embedding {
    identity: ServedIdentity,
    tokenizer: Tokenizer,
    /// A freshly built model with an empty key-value cache. Each request
    /// works on a clone; tensors are shared, so a clone is cheap.
    model: Model,
}

const EMBEDDING: Expected<'static> = Expected {
    stage: "embedding",
    backend: BACKEND,
    precision: PRECISION,
    dimensions: Some(DIMENSIONS),
    required: REQUIRED_FILES,
};

const RERANKING: Expected<'static> = Expected {
    stage: "reranking",
    backend: BACKEND,
    precision: PRECISION,
    dimensions: None,
    required: REQUIRED_FILES,
};

impl Qwen3Embedding {
    /// Verify `directory` against the configured manifest digest, then load.
    pub(crate) fn load(
        directory: &std::path::Path,
        manifest_sha256: &[u8; 32],
        max_input_tokens: usize,
    ) -> Result<Self, ArtifactError> {
        let verified = manifest::verify_directory(directory, manifest_sha256, EMBEDDING)?;
        Self::from_verified(verified, max_input_tokens)
    }

    #[cfg(test)]
    fn load_verified(directory: &std::path::Path, manifest: &[u8]) -> Result<Self, ArtifactError> {
        let digest = manifest::sha256(manifest);
        let verified = manifest::verify(directory, manifest, &digest, EMBEDDING)?;
        Self::from_verified(verified, 8192)
    }

    fn from_verified(
        mut verified: Verified,
        max_input_tokens: usize,
    ) -> Result<Self, ArtifactError> {
        let config = parse_config(&verified.take("config.json")?)?;
        if config.hidden_size != DIMENSIONS {
            return Err(ArtifactError::ManifestProfileMismatch);
        }
        check_positions(&config, max_input_tokens)?;
        let tokenizer = parse_tokenizer(&verified.take("tokenizer.json")?)?;
        let model = Model::new(&config, var_builder(verified.take("model.safetensors")?)?)
            .map_err(|_| ArtifactError::LoadFailed)?;
        Ok(Self {
            identity: ServedIdentity {
                model: verified.served_model,
                backend: BACKEND,
                retrieval_model: true,
                dimensions: Some(DIMENSIONS),
                manifest_sha256: Some(verified.manifest_sha256),
            },
            tokenizer,
            model,
        })
    }

    fn embed_one(&self, input: &str) -> Result<Vec<f32>, BackendError> {
        let ids = token_ids(&self.tokenizer, input)?;
        if ids.is_empty() {
            return Err(BackendError::Unavailable);
        }
        let mut model = self.model.clone();
        let hidden = model
            .forward(&ids_tensor(&ids)?, 0)
            .map_err(|_| BackendError::Unavailable)?;
        let pooled = last_token(&hidden).map_err(|_| BackendError::Unavailable)?;
        l2_normalize(pooled).ok_or(BackendError::Unavailable)
    }
}

impl EmbeddingBackend for Qwen3Embedding {
    fn identity(&self) -> &ServedIdentity {
        &self.identity
    }

    fn count_tokens(&self, input: &str) -> usize {
        // An input that cannot be tokenized counts as over any limit.
        token_ids(&self.tokenizer, input).map_or(usize::MAX, |ids| ids.len())
    }

    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, BackendError> {
        inputs.iter().map(|input| self.embed_one(input)).collect()
    }
}

pub(crate) struct Qwen3Reranking {
    identity: ServedIdentity,
    tokenizer: Tokenizer,
    prefix: Vec<u32>,
    suffix: Vec<u32>,
    yes: u32,
    no: u32,
    model: ModelForCausalLM,
}

/// The card's token assembly: prefix, then the content, then the suffix,
/// each tokenized on its own.
pub(crate) fn pair_ids(
    tokenizer: &Tokenizer,
    prefix: &[u32],
    suffix: &[u32],
    query: &str,
    document: &str,
) -> Result<Vec<u32>, BackendError> {
    let content = token_ids(tokenizer, &rerank_content(query, document))?;
    let mut ids = Vec::with_capacity(prefix.len() + content.len() + suffix.len());
    ids.extend_from_slice(prefix);
    ids.extend_from_slice(&content);
    ids.extend_from_slice(suffix);
    Ok(ids)
}

fn fixed_ids(tokenizer: &Tokenizer, text: &str) -> Result<Vec<u32>, ArtifactError> {
    tokenizer
        .encode(text, false)
        .map(|encoding| encoding.get_ids().to_vec())
        .map_err(|_| ArtifactError::LoadFailed)
}

impl Qwen3Reranking {
    /// Verify `directory` against the configured manifest digest, then load.
    pub(crate) fn load(
        directory: &std::path::Path,
        manifest_sha256: &[u8; 32],
        max_input_tokens: usize,
    ) -> Result<Self, ArtifactError> {
        let verified = manifest::verify_directory(directory, manifest_sha256, RERANKING)?;
        Self::from_verified(verified, max_input_tokens)
    }

    #[cfg(test)]
    fn load_verified(directory: &std::path::Path, manifest: &[u8]) -> Result<Self, ArtifactError> {
        let digest = manifest::sha256(manifest);
        let verified = manifest::verify(directory, manifest, &digest, RERANKING)?;
        Self::from_verified(verified, 8192)
    }

    fn from_verified(
        mut verified: Verified,
        max_input_tokens: usize,
    ) -> Result<Self, ArtifactError> {
        let config = parse_config(&verified.take("config.json")?)?;
        check_positions(&config, max_input_tokens)?;
        let tokenizer = parse_tokenizer(&verified.take("tokenizer.json")?)?;
        let prefix = fixed_ids(&tokenizer, RERANK_PREFIX)?;
        let suffix = fixed_ids(&tokenizer, RERANK_SUFFIX)?;
        let (Some(yes), Some(no)) = (tokenizer.token_to_id("yes"), tokenizer.token_to_id("no"))
        else {
            return Err(ArtifactError::LoadFailed);
        };
        if yes as usize >= config.vocab_size || no as usize >= config.vocab_size {
            return Err(ArtifactError::LoadFailed);
        }
        let model =
            ModelForCausalLM::new(&config, var_builder(verified.take("model.safetensors")?)?)
                .map_err(|_| ArtifactError::LoadFailed)?;
        Ok(Self {
            identity: ServedIdentity {
                model: verified.served_model,
                backend: BACKEND,
                retrieval_model: true,
                dimensions: None,
                manifest_sha256: Some(verified.manifest_sha256),
            },
            tokenizer,
            prefix,
            suffix,
            yes,
            no,
            model,
        })
    }

    fn score_one(&self, query: &str, document: &str) -> Result<f32, BackendError> {
        let ids = pair_ids(&self.tokenizer, &self.prefix, &self.suffix, query, document)?;
        let mut model = self.model.clone();
        let logits = model
            .forward(&ids_tensor(&ids)?, 0)
            .and_then(|logits| logits.flatten_all())
            .and_then(|logits| logits.to_vec1::<f32>())
            .map_err(|_| BackendError::Unavailable)?;
        yes_no_score(&logits, self.yes, self.no).ok_or(BackendError::Unavailable)
    }
}

impl RerankingBackend for Qwen3Reranking {
    fn identity(&self) -> &ServedIdentity {
        &self.identity
    }

    fn count_pair_tokens(&self, query: &str, document: &str) -> usize {
        pair_ids(&self.tokenizer, &self.prefix, &self.suffix, query, document)
            .map_or(usize::MAX, |ids| ids.len())
    }

    fn score(&self, query: &str, documents: &[String]) -> Result<Vec<f32>, BackendError> {
        documents
            .iter()
            .map(|document| self.score_one(query, document))
            .collect()
    }
}

/// Keep the reduction helper in one place for the tests below.
#[cfg(test)]
fn log_softmax_yes(logits: &[f32], yes: u32, no: u32) -> f32 {
    let pair = Tensor::new(&[logits[no as usize], logits[yes as usize]], &Device::Cpu).unwrap();
    candle_nn::ops::log_softmax(&pair, candle_core::D::Minus1)
        .unwrap()
        .to_vec1::<f32>()
        .unwrap()[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model card's `format_instruction` and Transformers prefix/suffix,
    /// transcribed from the pinned card.
    fn card_prompt(instruction: &str, query: &str, document: &str) -> String {
        let prefix = "<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be \"yes\" or \"no\".<|im_end|>\n<|im_start|>user\n";
        let suffix = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n";
        format!(
            "{prefix}<Instruct>: {instruction}\n<Query>: {query}\n<Document>: {document}{suffix}"
        )
    }

    #[test]
    fn rerank_template_with_server_prefix_equals_the_model_card_prompt() {
        let instruction =
            "Given a web search query, retrieve relevant passages that answer the query";
        // What Server sends as `query` when it applies the rerank instruction.
        let query = format!("<Instruct>: {instruction}\n<Query>: What is the capital of China?");
        let document = "The capital of China is Beijing.";
        let lens = format!(
            "{RERANK_PREFIX}{}{RERANK_SUFFIX}",
            rerank_content(&query, document)
        );
        assert_eq!(
            lens,
            card_prompt(instruction, "What is the capital of China?", document)
        );
        // Lens itself adds no instruction text.
        let bare = rerank_content("q", "d");
        assert_eq!(bare, "q\n<Document>: d");
        assert!(!RERANK_PREFIX.contains("Given a web search query"));
    }

    /// A tiny whitespace word-level tokenizer, so token assembly can be
    /// checked without the publisher's files.
    fn toy_tokenizer() -> Tokenizer {
        let vocab: Vec<String> = ["[UNK]", "P", "S", "q", "<Document>:", "d", "yes", "no"]
            .iter()
            .enumerate()
            .map(|(id, token)| format!("{:?}:{id}", token))
            .collect();
        let json = format!(
            r#"{{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{{"type":"WhitespaceSplit"}},"post_processor":null,"decoder":null,"model":{{"type":"WordLevel","vocab":{{{}}},"unk_token":"[UNK]"}}}}"#,
            vocab.join(",")
        );
        Tokenizer::from_bytes(json.as_bytes()).unwrap()
    }

    #[test]
    fn pair_tokens_are_prefix_then_content_then_suffix() {
        let tokenizer = toy_tokenizer();
        let prefix = fixed_ids(&tokenizer, "P").unwrap();
        let suffix = fixed_ids(&tokenizer, "S").unwrap();
        let ids = pair_ids(&tokenizer, &prefix, &suffix, "q", "d").unwrap();
        assert_eq!(ids, [1, 3, 4, 5, 2]);
        assert_eq!(tokenizer.token_to_id("yes"), Some(6));
    }

    #[test]
    fn pools_the_last_position() {
        let hidden = Tensor::new(
            &[[[1f32, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]]],
            &Device::Cpu,
        )
        .unwrap();
        assert_eq!(last_token(&hidden).unwrap(), [7.0, 8.0, 9.0]);
    }

    #[test]
    fn normalizes_to_unit_length_and_rejects_degenerate_vectors() {
        let unit = l2_normalize(vec![3.0, 4.0]).unwrap();
        assert_eq!(unit, [0.6, 0.8]);
        assert_eq!(l2_normalize(vec![0.0, 0.0]), None);
        assert_eq!(l2_normalize(vec![f32::NAN, 1.0]), None);
        assert_eq!(l2_normalize(vec![f32::INFINITY, 1.0]), None);
    }

    #[test]
    fn score_is_the_log_odds_of_the_cards_yes_probability() {
        let logits = [0.0f32, 2.5, -1.25, 9.0];
        let (yes, no) = (1, 2);
        let score = yes_no_score(&logits, yes, no).unwrap();
        assert_eq!(score, 3.75);
        // Card: P(yes) = exp(log_softmax([no, yes])[1]) = sigmoid(score).
        let probability = log_softmax_yes(&logits, yes, no).exp();
        let sigmoid = 1.0 / (1.0 + (-score).exp());
        assert!((probability - sigmoid).abs() < 1e-6);
        assert_eq!(yes_no_score(&logits, 9, no), None);
    }

    /// Run against the publisher's weights. Opt in with
    /// `LENS_QWEN3_TEST_DIR=<dir> cargo test --release --features qwen3 -- --ignored --nocapture`,
    /// where `<dir>` holds `qwen3-embedding-0.6b/` and `qwen3-reranker-0.6b/`
    /// with at least the three required files each. The test writes no file:
    /// it builds each manifest in memory from the files it finds.
    #[test]
    #[ignore = "needs the publisher's weights in LENS_QWEN3_TEST_DIR"]
    fn matches_the_model_cards_on_real_weights() {
        let Some(root) = std::env::var_os("LENS_QWEN3_TEST_DIR") else {
            eprintln!("LENS_QWEN3_TEST_DIR is not set; skipping");
            return;
        };
        let root = std::path::PathBuf::from(root);
        let started = std::time::Instant::now();
        let embedding = load_for_test(&root.join("qwen3-embedding-0.6b"), "embedding", |d, m| {
            Qwen3Embedding::load_verified(d, m)
        });
        let rerank = load_for_test(&root.join("qwen3-reranker-0.6b"), "reranking", |d, m| {
            Qwen3Reranking::load_verified(d, m)
        });
        eprintln!("load both stages: {:.1?}", started.elapsed());
        assert!(embedding.identity().retrieval_model);

        // Embedding card, Transformers usage: expected cosine scores.
        let task = "Given a web search query, retrieve relevant passages that answer the query";
        let inputs: Vec<String> = [
            format!("Instruct: {task}\nQuery:What is the capital of China?"),
            format!("Instruct: {task}\nQuery:Explain gravity"),
            "The capital of China is Beijing.".to_owned(),
            "Gravity is a force that attracts two bodies towards each other. It gives weight to physical objects and is responsible for the movement of planets around the sun.".to_owned(),
        ]
        .into();
        let started = std::time::Instant::now();
        let vectors = embedding.embed(&inputs).unwrap();
        eprintln!("embed 4 inputs: {:.1?}", started.elapsed());
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let scores = [
            dot(&vectors[0], &vectors[2]),
            dot(&vectors[0], &vectors[3]),
            dot(&vectors[1], &vectors[2]),
            dot(&vectors[1], &vectors[3]),
        ];
        eprintln!("embedding cosine scores: {scores:?}");
        for (found, card) in scores.iter().zip([0.7646, 0.1414, 0.1355, 0.6000]) {
            assert!((found - card).abs() < 0.01, "{found} vs card {card}");
        }
        for vector in &vectors {
            assert_eq!(vector.len(), DIMENSIONS);
            assert!((dot(vector, vector) - 1.0).abs() < 1e-4);
        }

        // Reranker card, Sentence Transformers usage: logit differences.
        let query = format!("<Instruct>: {task}\n<Query>: What is the capital of China?");
        let documents = [inputs[2].clone(), inputs[3].clone()];
        let started = std::time::Instant::now();
        let scores = rerank.score(&query, &documents).unwrap();
        eprintln!("rerank 2 pairs: {:.1?}", started.elapsed());
        eprintln!(
            "rerank scores: {scores:?}; pair tokens {}",
            rerank.count_pair_tokens(&query, &documents[0])
        );
        // The card's values are multiples of 1/8 (reduced precision). Far in
        // the tail a small logit difference is a large log-odds difference,
        // so compare the card's probability, P(yes) = sigmoid(score).
        let sigmoid = |x: f32| 1.0 / (1.0 + (-x).exp());
        assert!(scores[0] > 0.0 && scores[1] < 0.0);
        for (found, card) in scores.iter().zip([7.625f32, -11.375]) {
            let gap = (sigmoid(*found) - sigmoid(card)).abs();
            assert!(gap < 1e-3, "{found} vs card {card}");
        }

        // Indicative timing for a larger pair, single sequence on CPU.
        let long = "The orchard grows apples and pears in a temperate climate. ".repeat(40);
        let tokens = rerank.count_pair_tokens(&query, &long);
        let started = std::time::Instant::now();
        rerank.score(&query, &[long]).unwrap();
        eprintln!(
            "rerank 1 pair of {tokens} tokens: {:.1?}",
            started.elapsed()
        );
    }

    fn load_for_test<T>(
        directory: &std::path::Path,
        stage: &str,
        load: impl FnOnce(&std::path::Path, &[u8]) -> Result<T, ArtifactError>,
    ) -> T {
        let entries: Vec<String> = REQUIRED_FILES
            .iter()
            .map(|name| {
                let bytes = std::fs::read(directory.join(name)).unwrap();
                format!(
                    r#"{{"path":"{name}","size":{},"sha256":"{}"}}"#,
                    bytes.len(),
                    manifest::tests::hex(&manifest::sha256(&bytes))
                )
            })
            .collect();
        let dimensions = if stage == "embedding" {
            format!(r#""dimensions":{DIMENSIONS},"#)
        } else {
            String::new()
        };
        let manifest = format!(
            r#"{{"format":"{}","stage":"{stage}","backend":"{BACKEND}","served_model":"qwen3-{stage}-test","precision":"{PRECISION}",{dimensions}"upstream":{{"repository":"local","revision":"test"}},"files":[{}]}}"#,
            manifest::FORMAT,
            entries.join(",")
        );
        match load(directory, manifest.as_bytes()) {
            Ok(backend) => backend,
            Err(error) => panic!("load failed: {error}"),
        }
    }
}
