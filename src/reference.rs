// SPDX-License-Identifier: Apache-2.0

//! Deterministic reference backend for tests and protocol qualification.
//!
//! This is NOT a retrieval model. It loads no weights, learns nothing and
//! makes no claim about relevance. Its vectors are hashed token counts and
//! its scores are query-token overlap. It exists so that contributors and the
//! Server consumer can exercise the complete wire profile, limits and refusals
//! without a GPU, model download or credentials. Readiness and discovery
//! report it with `"backend":"reference"` and `"retrieval_model":false`.
//!
//! The algorithm is fixed so an independent implementation can reproduce
//! every value:
//!
//! - Tokens: split the input on every `char` for which Rust's
//!   `char::is_alphanumeric` is false, drop empty pieces and lowercase each
//!   piece with `str::to_lowercase`.
//! - Hash: 64-bit FNV-1a over the token's UTF-8 bytes.
//! - Embedding: start from a zero vector of the configured dimension. For each
//!   token add 1.0 at `hash % dimension`. An input with no token adds 1.0 at
//!   `FNV-1a("") % dimension`. Divide by the Euclidean norm.
//! - Reranking: the score of a document is the number of distinct query tokens
//!   present in the document divided by the number of distinct query tokens,
//!   or 0.0 when the query has no token.
//! - Token counts: the number of tokens above. A rerank pair counts the query
//!   tokens plus the document tokens.

use std::collections::BTreeSet;

use crate::backend::{BackendError, EmbeddingBackend, RerankingBackend, ServedIdentity};

/// Backend family label reported by readiness and discovery.
pub(crate) const BACKEND: &str = "reference";
/// Served reranking model name.
pub(crate) const RERANKING_MODEL: &str = "lens-reference/rerank-v1";

/// Served embedding model name. The dimension is part of the identity.
pub(crate) fn embedding_model(dimensions: usize) -> String {
    format!("lens-reference/embedding-v1-{dimensions}d")
}

fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|piece| !piece.is_empty())
        .map(str::to_lowercase)
}

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub(crate) struct ReferenceEmbedding {
    identity: ServedIdentity,
    dimensions: usize,
}

impl ReferenceEmbedding {
    /// `dimensions` must be at least 1; configuration enforces the range.
    pub(crate) fn new(dimensions: usize) -> Self {
        Self {
            identity: ServedIdentity {
                model: embedding_model(dimensions),
                backend: BACKEND,
                retrieval_model: false,
                dimensions: Some(dimensions),
                manifest_sha256: None,
            },
            dimensions,
        }
    }

    fn bucket(&self, bytes: &[u8]) -> usize {
        // The remainder is below `dimensions`, which is a usize.
        (fnv1a64(bytes) % self.dimensions as u64) as usize
    }

    fn embed_one(&self, input: &str) -> Vec<f32> {
        let mut counts = vec![0.0f64; self.dimensions];
        let mut any = false;
        for token in tokens(input) {
            counts[self.bucket(token.as_bytes())] += 1.0;
            any = true;
        }
        if !any {
            counts[self.bucket(b"")] = 1.0;
        }
        let norm = counts.iter().map(|value| value * value).sum::<f64>().sqrt();
        counts.iter().map(|value| (value / norm) as f32).collect()
    }
}

impl EmbeddingBackend for ReferenceEmbedding {
    fn identity(&self) -> &ServedIdentity {
        &self.identity
    }

    fn count_tokens(&self, input: &str) -> usize {
        tokens(input).count()
    }

    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, BackendError> {
        Ok(inputs.iter().map(|input| self.embed_one(input)).collect())
    }
}

pub(crate) struct ReferenceReranking {
    identity: ServedIdentity,
}

impl ReferenceReranking {
    pub(crate) fn new() -> Self {
        Self {
            identity: ServedIdentity {
                model: RERANKING_MODEL.to_owned(),
                backend: BACKEND,
                retrieval_model: false,
                dimensions: None,
                manifest_sha256: None,
            },
        }
    }
}

impl RerankingBackend for ReferenceReranking {
    fn identity(&self) -> &ServedIdentity {
        &self.identity
    }

    fn count_pair_tokens(&self, query: &str, document: &str) -> usize {
        tokens(query).count() + tokens(document).count()
    }

    fn score(&self, query: &str, documents: &[String]) -> Result<Vec<f32>, BackendError> {
        let query: BTreeSet<String> = tokens(query).collect();
        Ok(documents
            .iter()
            .map(|document| {
                if query.is_empty() {
                    return 0.0;
                }
                let document: BTreeSet<String> = tokens(document).collect();
                let shared = query.intersection(&document).count();
                (shared as f64 / query.len() as f64) as f32
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_published_test_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn tokenizes_on_non_alphanumeric_characters_and_lowercases() {
        let found: Vec<String> = tokens("An ORCHARD, grows-fruit! Äpfel 42").collect();
        assert_eq!(found, ["an", "orchard", "grows", "fruit", "äpfel", "42"]);
        assert_eq!(tokens(" ?! ").count(), 0);
    }

    #[test]
    fn embeds_hashed_token_counts_with_unit_norm() {
        let backend = ReferenceEmbedding::new(8);
        // FNV-1a("a") % 8 == 4, so "a a" is the unit vector e4.
        let vectors = backend.embed(&["A a".to_owned()]).unwrap();
        let mut expected = vec![0.0f32; 8];
        expected[4] = 1.0;
        assert_eq!(vectors, [expected]);
        // An input without a token maps to FNV-1a("") % 8 == 5.
        let empty = backend.embed(&[String::new()]).unwrap();
        assert_eq!(empty[0][5], 1.0);
        let other = backend.embed(&["orchard fruit".to_owned()]).unwrap();
        let norm: f32 = other[0].iter().map(|value| value * value).sum();
        assert!((norm - 1.0).abs() < 1e-6);
        assert_eq!(backend.identity().model, "lens-reference/embedding-v1-8d");
        assert!(!backend.identity().retrieval_model);
    }

    #[test]
    fn scores_distinct_query_token_overlap() {
        let backend = ReferenceReranking::new();
        let documents = [
            "An orchard grows fruit.".to_owned(),
            "A library lends books.".to_owned(),
            "orchard".to_owned(),
        ];
        let scores = backend
            .score("What grows in an orchard orchard?", &documents)
            .unwrap();
        // Distinct query tokens: what, grows, in, an, orchard.
        assert_eq!(scores, [0.6, 0.0, 0.2]);
        assert_eq!(backend.score("?", &documents).unwrap(), [0.0; 3]);
        assert_eq!(backend.count_pair_tokens("a b", "c"), 3);
    }
}
