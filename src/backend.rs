// SPDX-License-Identifier: Apache-2.0

//! Stage backend interfaces. A backend value exists only after it has loaded;
//! the server publishes it as ready only after its startup probe passes.

use serde::Serialize;

/// Identity a loaded stage serves under. Readiness and model discovery
/// report it so Server's enrollment probe can compare it with the binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServedIdentity {
    /// Exact served model name. Requests must name it.
    pub(crate) model: String,
    /// Backend family, for example `reference`.
    pub(crate) backend: &'static str,
    /// False for test backends that do not implement a retrieval model.
    pub(crate) retrieval_model: bool,
    /// Embedding dimension. `None` for reranking.
    pub(crate) dimensions: Option<usize>,
    /// Canonical digest of the verified artifact manifest. `None` when the
    /// backend loads no artifacts.
    pub(crate) manifest_sha256: Option<[u8; 32]>,
}

/// Content-free backend failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendError {
    /// The loaded runtime could not run the work.
    #[cfg_attr(
        not(any(test, feature = "qwen3")),
        expect(
            dead_code,
            reason = "model runtime adapters return it; the reference backend cannot fail"
        )
    )]
    Unavailable,
}

pub(crate) trait EmbeddingBackend: Send + Sync {
    fn identity(&self) -> &ServedIdentity;
    /// Count the tokens of the exact effective input, as the backend
    /// would see it, before any work is admitted.
    fn count_tokens(&self, input: &str) -> usize;
    /// Return one vector per input, in input order.
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, BackendError>;
}

pub(crate) trait RerankingBackend: Send + Sync {
    fn identity(&self) -> &ServedIdentity;
    /// Count the tokens of one effective query and document pair,
    /// including any template the backend applies.
    fn count_pair_tokens(&self, query: &str, document: &str) -> usize;
    /// Return one relevance score per document, in input order. The server
    /// builds the permutation, so a backend cannot drop or repeat an index.
    fn score(&self, query: &str, documents: &[String]) -> Result<Vec<f32>, BackendError>;
}

#[derive(Serialize)]
pub(crate) struct RerankHit {
    pub(crate) index: usize,
    pub(crate) relevance_score: f32,
}

/// Order every input index by descending score, breaking ties by original
/// position. Callers validate that every score is finite first.
pub(crate) fn rank(scores: &[f32]) -> Vec<RerankHit> {
    let mut hits: Vec<RerankHit> = scores
        .iter()
        .enumerate()
        .map(|(index, &relevance_score)| RerankHit {
            index,
            relevance_score,
        })
        .collect();
    // Numeric comparison treats -0.0 and 0.0 as a tie, unlike `total_cmp`.
    hits.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.index.cmp(&b.index))
    });
    hits
}

#[cfg(test)]
mod tests {
    use super::rank;

    #[test]
    fn rank_sorts_descending_and_keeps_tied_positions_stable() {
        let order: Vec<usize> = rank(&[0.5, 1.0, 0.5, 0.0, 1.0, -0.0])
            .iter()
            .map(|hit| hit.index)
            .collect();
        assert_eq!(order, [1, 4, 0, 2, 3, 5]);
    }
}
