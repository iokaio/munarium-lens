// SPDX-License-Identifier: Apache-2.0

use serde::Serialize;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ServedIdentity {
    pub(crate) model: String,
    pub(crate) manifest_sha256: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct InferenceUnavailable;

pub(crate) trait EmbeddingBackend {
    fn identity(&self) -> Result<ServedIdentity, InferenceUnavailable>;
    fn embed(&self, model: &str, input: &[String]) -> Result<Vec<Vec<f32>>, InferenceUnavailable>;
}

#[derive(Serialize)]
pub(crate) struct RerankHit {
    pub(crate) index: usize,
    pub(crate) relevance_score: f32,
}

pub(crate) trait RerankingBackend {
    fn identity(&self) -> Result<ServedIdentity, InferenceUnavailable>;
    fn rerank(
        &self,
        model: &str,
        query: &str,
        documents: &[String],
    ) -> Result<Vec<RerankHit>, InferenceUnavailable>;
}

pub(crate) struct UnavailableBackend;

impl EmbeddingBackend for UnavailableBackend {
    fn identity(&self) -> Result<ServedIdentity, InferenceUnavailable> {
        Err(InferenceUnavailable)
    }

    fn embed(&self, _: &str, _: &[String]) -> Result<Vec<Vec<f32>>, InferenceUnavailable> {
        Err(InferenceUnavailable)
    }
}

impl RerankingBackend for UnavailableBackend {
    fn identity(&self) -> Result<ServedIdentity, InferenceUnavailable> {
        Err(InferenceUnavailable)
    }

    fn rerank(
        &self,
        _: &str,
        _: &str,
        _: &[String],
    ) -> Result<Vec<RerankHit>, InferenceUnavailable> {
        Err(InferenceUnavailable)
    }
}
