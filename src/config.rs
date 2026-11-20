// SPDX-License-Identifier: Apache-2.0

//! Service configuration from `LENS_*` environment variables.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

/// Backend selected for one stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StageBackend {
    /// The stage is not served. Its route returns `inference_unavailable`.
    #[default]
    Disabled,
    /// The deterministic reference backend. Not a retrieval model.
    Reference,
    /// Qwen3-Embedding or Qwen3-Reranker on Candle, CPU. Present only in a
    /// build with the `qwen3` feature.
    #[cfg(feature = "qwen3")]
    Qwen3,
}

/// A verified artifact directory for a model-backed stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageArtifacts {
    /// Operator-supplied directory holding `lens-manifest.json` and the files
    /// it lists. Mount it read-only.
    pub directory: PathBuf,
    /// Expected SHA-256 of the manifest file's exact bytes.
    pub manifest_sha256: [u8; 32],
}

/// Request, batch and admission bounds, applied per stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Largest accepted request body in bytes.
    pub max_body_bytes: usize,
    /// Most embedding inputs or rerank documents in one request.
    pub max_items: usize,
    /// Most tokens in one embedding input or one query and document pair.
    pub max_input_tokens: usize,
    /// Most tokens across one request.
    pub max_batch_tokens: usize,
    /// Concurrent executions per stage.
    pub max_in_flight: usize,
    /// Requests allowed to wait for an execution slot per stage.
    pub max_queue: usize,
    /// Longest a request waits for an execution slot.
    pub queue_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body_bytes: 256 * 1024,
            max_items: 64,
            max_input_tokens: 512,
            max_batch_tokens: 16_384,
            max_in_flight: 4,
            max_queue: 16,
            queue_timeout: Duration::from_millis(1_000),
        }
    }
}

/// Complete service configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub listen: SocketAddr,
    /// Permit a non-loopback listener. Lens has no service authentication
    /// yet, so this is only for an isolated container network whose published
    /// port is bound to the host loopback interface.
    pub allow_unauthenticated_non_loopback: bool,
    pub embedding: StageBackend,
    pub reranking: StageBackend,
    /// Dimension of the reference embedding backend.
    pub reference_dimensions: usize,
    /// Artifacts of a model-backed embedding stage.
    pub embedding_artifacts: Option<StageArtifacts>,
    /// Artifacts of a model-backed reranking stage.
    pub reranking_artifacts: Option<StageArtifacts>,
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
            allow_unauthenticated_non_loopback: false,
            embedding: StageBackend::Disabled,
            reranking: StageBackend::Disabled,
            reference_dimensions: 1024,
            embedding_artifacts: None,
            reranking_artifacts: None,
            limits: Limits::default(),
        }
    }
}

/// Invalid configuration. Names the variable, never its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub variable: &'static str,
    pub expected: &'static str,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: expected {}", self.variable, self.expected)
    }
}

impl std::error::Error for ConfigError {}

const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIMENSIONS: usize = 4096;

fn number(
    get: &impl Fn(&str) -> Option<String>,
    variable: &'static str,
    min: usize,
    max: usize,
    expected: &'static str,
    default: usize,
) -> Result<usize, ConfigError> {
    let Some(value) = get(variable) else {
        return Ok(default);
    };
    match value.trim().parse::<usize>() {
        Ok(parsed) if (min..=max).contains(&parsed) => Ok(parsed),
        _ => Err(ConfigError { variable, expected }),
    }
}

#[cfg(feature = "qwen3")]
const BACKENDS: &str = "disabled, reference or qwen3";
#[cfg(not(feature = "qwen3"))]
const BACKENDS: &str = "disabled or reference";

fn backend(
    get: &impl Fn(&str) -> Option<String>,
    variable: &'static str,
) -> Result<StageBackend, ConfigError> {
    match get(variable).as_deref().map(str::trim) {
        None | Some("disabled") => Ok(StageBackend::Disabled),
        Some("reference") => Ok(StageBackend::Reference),
        #[cfg(feature = "qwen3")]
        Some("qwen3") => Ok(StageBackend::Qwen3),
        Some(_) => Err(ConfigError {
            variable,
            expected: BACKENDS,
        }),
    }
}

/// Parse a lowercase 64-character hex SHA-256.
pub(crate) fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut digest = [0u8; 32];
    for (index, [high, low]) in bytes.as_chunks::<2>().0.iter().enumerate() {
        digest[index] = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Some(digest)
}

const DIGEST_EXPECTED: &str = "the manifest's lowercase hex SHA-256";

/// Read a stage's artifact directory and manifest digest. Both or neither
/// must be set, and both are required when `required` is true.
fn artifacts(
    get: &impl Fn(&str) -> Option<String>,
    directory_variable: &'static str,
    digest_variable: &'static str,
    required: bool,
) -> Result<Option<StageArtifacts>, ConfigError> {
    let directory = get(directory_variable).filter(|value| !value.trim().is_empty());
    let digest = get(digest_variable);
    match (directory, digest) {
        (None, None) if !required => Ok(None),
        (None, _) => Err(ConfigError {
            variable: directory_variable,
            expected: "an artifact directory for the model-backed stage",
        }),
        (Some(_), None) => Err(ConfigError {
            variable: digest_variable,
            expected: DIGEST_EXPECTED,
        }),
        (Some(directory), Some(digest)) => Ok(Some(StageArtifacts {
            directory: PathBuf::from(directory.trim()),
            manifest_sha256: parse_sha256(digest.trim()).ok_or(ConfigError {
                variable: digest_variable,
                expected: DIGEST_EXPECTED,
            })?,
        })),
    }
}

fn is_model_backed(backend: StageBackend) -> bool {
    match backend {
        StageBackend::Disabled | StageBackend::Reference => false,
        #[cfg(feature = "qwen3")]
        StageBackend::Qwen3 => true,
    }
}

impl Config {
    /// Read configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Read configuration through `get`, which returns a variable's value.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let limits = Limits::default();
        let host = match get("LENS_HOST") {
            None => defaults.listen.ip(),
            Some(value) => value.trim().parse::<IpAddr>().map_err(|_| ConfigError {
                variable: "LENS_HOST",
                expected: "an IP address",
            })?,
        };
        let port = number(&get, "LENS_PORT", 0, 65_535, "a port number", 8787)?;
        let allow = match get("LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK").as_deref() {
            None | Some("0") => false,
            Some("1") => true,
            Some(_) => {
                return Err(ConfigError {
                    variable: "LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK",
                    expected: "0 or 1",
                });
            }
        };
        let queue_timeout_ms = number(
            &get,
            "LENS_QUEUE_TIMEOUT_MS",
            1,
            60_000,
            "milliseconds from 1 to 60000",
            1_000,
        )?;
        let embedding = backend(&get, "LENS_EMBEDDING_BACKEND")?;
        let reranking = backend(&get, "LENS_RERANKING_BACKEND")?;
        Ok(Self {
            // The range check above keeps the port within u16.
            listen: SocketAddr::new(host, port as u16),
            allow_unauthenticated_non_loopback: allow,
            embedding,
            reranking,
            embedding_artifacts: artifacts(
                &get,
                "LENS_EMBEDDING_ARTIFACTS",
                "LENS_EMBEDDING_MANIFEST_SHA256",
                is_model_backed(embedding),
            )?,
            reranking_artifacts: artifacts(
                &get,
                "LENS_RERANKING_ARTIFACTS",
                "LENS_RERANKING_MANIFEST_SHA256",
                is_model_backed(reranking),
            )?,
            reference_dimensions: number(
                &get,
                "LENS_REFERENCE_DIMENSIONS",
                1,
                MAX_DIMENSIONS,
                "an integer from 1 to 4096",
                defaults.reference_dimensions,
            )?,
            limits: Limits {
                max_body_bytes: number(
                    &get,
                    "LENS_MAX_BODY_BYTES",
                    1,
                    MAX_BODY_BYTES,
                    "bytes from 1 to 16777216",
                    limits.max_body_bytes,
                )?,
                max_items: number(
                    &get,
                    "LENS_MAX_ITEMS",
                    1,
                    4_096,
                    "an integer from 1 to 4096",
                    limits.max_items,
                )?,
                max_input_tokens: number(
                    &get,
                    "LENS_MAX_INPUT_TOKENS",
                    1,
                    1_048_576,
                    "an integer from 1 to 1048576",
                    limits.max_input_tokens,
                )?,
                max_batch_tokens: number(
                    &get,
                    "LENS_MAX_BATCH_TOKENS",
                    1,
                    16_777_216,
                    "an integer from 1 to 16777216",
                    limits.max_batch_tokens,
                )?,
                max_in_flight: number(
                    &get,
                    "LENS_MAX_IN_FLIGHT",
                    1,
                    1_024,
                    "an integer from 1 to 1024",
                    limits.max_in_flight,
                )?,
                max_queue: number(
                    &get,
                    "LENS_MAX_QUEUE",
                    0,
                    65_536,
                    "an integer from 0 to 65536",
                    limits.max_queue,
                )?,
                queue_timeout: Duration::from_millis(queue_timeout_ms as u64),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        Config::from_lookup(move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        })
    }

    #[test]
    fn defaults_to_loopback_with_both_stages_disabled() {
        assert_eq!(lookup(&[]).unwrap(), Config::default());
        assert!(Config::default().listen.ip().is_loopback());
    }

    #[test]
    fn reads_stage_backends_and_limits() {
        let config = lookup(&[
            ("LENS_PORT", "9000"),
            ("LENS_EMBEDDING_BACKEND", "reference"),
            ("LENS_RERANKING_BACKEND", "disabled"),
            ("LENS_REFERENCE_DIMENSIONS", "256"),
            ("LENS_MAX_ITEMS", "8"),
            ("LENS_MAX_QUEUE", "0"),
            ("LENS_QUEUE_TIMEOUT_MS", "250"),
        ])
        .unwrap();
        assert_eq!(config.listen.port(), 9000);
        assert_eq!(config.embedding, StageBackend::Reference);
        assert_eq!(config.reranking, StageBackend::Disabled);
        assert_eq!(config.reference_dimensions, 256);
        assert_eq!(config.limits.max_items, 8);
        assert_eq!(config.limits.max_queue, 0);
        assert_eq!(config.limits.queue_timeout, Duration::from_millis(250));
    }

    #[test]
    fn reads_artifacts_in_pairs_with_a_lowercase_digest() {
        let digest = "ab".repeat(32);
        let config = lookup(&[
            ("LENS_EMBEDDING_ARTIFACTS", "/srv/lens/embedding"),
            ("LENS_EMBEDDING_MANIFEST_SHA256", &digest),
        ])
        .unwrap();
        let artifacts = config.embedding_artifacts.unwrap();
        assert_eq!(artifacts.directory, PathBuf::from("/srv/lens/embedding"));
        assert_eq!(artifacts.manifest_sha256, [0xab; 32]);
        assert_eq!(config.reranking_artifacts, None);
        let upper = "AB".repeat(32);
        let error = lookup(&[
            ("LENS_EMBEDDING_ARTIFACTS", "/srv/lens/embedding"),
            ("LENS_EMBEDDING_MANIFEST_SHA256", &upper),
        ])
        .unwrap_err();
        assert_eq!(error.variable, "LENS_EMBEDDING_MANIFEST_SHA256");
        assert!(!error.to_string().contains(&upper));
        // Each half of a pair needs the other; errors carry no value.
        let error = lookup(&[("LENS_RERANKING_ARTIFACTS", "/srv/lens/reranker")]).unwrap_err();
        assert_eq!(error.variable, "LENS_RERANKING_MANIFEST_SHA256");
        assert!(!error.to_string().contains("/srv/lens"));
        let error = lookup(&[("LENS_EMBEDDING_MANIFEST_SHA256", &digest)]).unwrap_err();
        assert_eq!(error.variable, "LENS_EMBEDDING_ARTIFACTS");
    }

    #[cfg(not(feature = "qwen3"))]
    #[test]
    fn refuses_the_qwen3_backend_in_a_build_without_it() {
        let error = lookup(&[("LENS_EMBEDDING_BACKEND", "qwen3")]).unwrap_err();
        assert_eq!(error.variable, "LENS_EMBEDDING_BACKEND");
        assert_eq!(error.expected, "disabled or reference");
    }

    #[cfg(feature = "qwen3")]
    #[test]
    fn requires_artifacts_for_a_qwen3_stage() {
        let error = lookup(&[("LENS_RERANKING_BACKEND", "qwen3")]).unwrap_err();
        assert_eq!(error.variable, "LENS_RERANKING_ARTIFACTS");
        let error = lookup(&[
            ("LENS_RERANKING_BACKEND", "qwen3"),
            ("LENS_RERANKING_ARTIFACTS", "/srv/lens/reranker"),
        ])
        .unwrap_err();
        assert_eq!(error.variable, "LENS_RERANKING_MANIFEST_SHA256");
        let digest = "0f".repeat(32);
        let config = lookup(&[
            ("LENS_RERANKING_BACKEND", "qwen3"),
            ("LENS_RERANKING_ARTIFACTS", "/srv/lens/reranker"),
            ("LENS_RERANKING_MANIFEST_SHA256", &digest),
        ])
        .unwrap();
        assert_eq!(config.reranking, StageBackend::Qwen3);
    }

    #[test]
    fn names_the_invalid_variable_without_its_value() {
        for (name, value) in [
            ("LENS_EMBEDDING_BACKEND", "onnx"),
            ("LENS_REFERENCE_DIMENSIONS", "99999"),
            ("LENS_MAX_IN_FLIGHT", "-7"),
            ("LENS_PORT", "70000"),
            ("LENS_HOST", "localhost"),
            ("LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK", "yes"),
        ] {
            let error = lookup(&[(name, value)]).unwrap_err();
            assert_eq!(error.variable, name);
            assert!(!error.to_string().contains(value));
        }
        for name in [
            "LENS_REFERENCE_DIMENSIONS",
            "LENS_MAX_ITEMS",
            "LENS_MAX_IN_FLIGHT",
        ] {
            assert_eq!(lookup(&[(name, "0")]).unwrap_err().variable, name);
        }
    }
}
