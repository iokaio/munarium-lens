// SPDX-License-Identifier: Apache-2.0

//! Service configuration from `LENS_*` environment variables.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Backend selected for one stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StageBackend {
    /// The stage is not served. Its route returns `inference_unavailable`.
    #[default]
    Disabled,
    /// The deterministic reference backend. Not a retrieval model.
    Reference,
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

fn backend(
    get: &impl Fn(&str) -> Option<String>,
    variable: &'static str,
) -> Result<StageBackend, ConfigError> {
    match get(variable).as_deref().map(str::trim) {
        None | Some("disabled") => Ok(StageBackend::Disabled),
        Some("reference") => Ok(StageBackend::Reference),
        Some(_) => Err(ConfigError {
            variable,
            expected: "disabled or reference",
        }),
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
        Ok(Self {
            // The range check above keeps the port within u16.
            listen: SocketAddr::new(host, port as u16),
            allow_unauthenticated_non_loopback: allow,
            embedding: backend(&get, "LENS_EMBEDDING_BACKEND")?,
            reranking: backend(&get, "LENS_RERANKING_BACKEND")?,
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
    fn names_the_invalid_variable_without_its_value() {
        for (name, value) in [
            ("LENS_EMBEDDING_BACKEND", "qwen"),
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
