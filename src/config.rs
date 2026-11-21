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

/// A pinned model bundle that Lens fetches for a stage at startup. Only a
/// build with the `fetch` feature accepts one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageBundle {
    /// Bundle name: the manifest's `served_model` and the bundle's directory
    /// in both distribution locations.
    pub bundle: String,
    /// Pinned Hugging Face repository commit: 40 lowercase hex characters.
    pub revision: String,
}

/// Where fetched bundles are written, and how long one file may take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCache {
    /// Writable directory outside the source tree. Each bundle is written to
    /// `<directory>/<bundle>`.
    pub directory: PathBuf,
    /// Longest one file's download may take, redirects included.
    pub file_timeout: Duration,
    /// The egress proxy the fetcher tunnels through, from the standard
    /// proxy variables. A transport route only: it never changes which hosts
    /// are fetched.
    pub proxy: Option<FetchProxy>,
}

/// An HTTP `CONNECT` proxy for the fetcher, read from `HTTPS_PROXY` or
/// `https_proxy`, with exclusions from `NO_PROXY` or `no_proxy`.
///
/// The fetcher opens a tunnel through it and runs TLS end to end to the
/// fixed download host inside, so certificate validation and the redirect
/// allowlist apply to the target hosts exactly as on a direct connection.
/// Only `http://` proxy URLs are accepted (TLS to the proxy itself and SOCKS
/// are not). `ALL_PROXY` and `HTTP_PROXY` are deliberately not read: every
/// download is HTTPS, and `ALL_PROXY` often names a SOCKS proxy.
///
/// `Debug` never shows the credentials, and nothing else prints them.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchProxy {
    /// Host as written in the URL; an IPv6 address keeps its brackets.
    host: String,
    port: u16,
    /// Percent-encoded `user[:password]`, exactly as given.
    userinfo: Option<String>,
    /// Lowercase host names, domain suffixes or IP addresses, or `*`.
    no_proxy: Vec<String>,
}

impl fmt::Debug for FetchProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchProxy")
            .field("host", &self.host)
            .field("port", &self.port)
            .field(
                "credentials",
                &if self.userinfo.is_some() {
                    "<redacted>"
                } else {
                    "none"
                },
            )
            .field("no_proxy", &self.no_proxy)
            .finish()
    }
}

const PROXY_EXPECTED: &str = "an http:// proxy URL such as http://<host>:<port>, with \
any credentials percent-encoded (HTTPS-to-proxy and SOCKS are not supported)";

impl FetchProxy {
    /// Read the proxy settings from the process environment. `None` when no
    /// HTTPS proxy is set.
    pub fn from_env() -> Result<Option<Self>, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Read the proxy settings through `get`. Always reads exactly
    /// `HTTPS_PROXY`, `https_proxy`, `NO_PROXY` and `no_proxy`.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, ConfigError> {
        proxy(&get)
    }

    /// The proxy host, as written in the URL.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether the proxy URL carries credentials.
    pub fn has_credentials(&self) -> bool {
        self.userinfo.is_some()
    }

    /// The proxy URL for the HTTP client, including any credentials. Pass it
    /// only to the client; never print or log it.
    pub fn client_url(&self) -> String {
        match &self.userinfo {
            Some(userinfo) => format!("http://{userinfo}@{}:{}", self.host, self.port),
            None => format!("http://{}:{}", self.host, self.port),
        }
    }

    /// Whether `NO_PROXY` sends a connection to `host` directly: `*`, the
    /// same name or address, or a subdomain of a listed domain (a leading
    /// `.` or `*.` is optional). CIDR ranges and ports are not supported, so
    /// such an entry never matches and the proxy is used.
    pub fn bypasses(&self, host: &str) -> bool {
        let host = normalize_host(host);
        self.no_proxy.iter().any(|entry| {
            entry == "*"
                || *entry == host
                || host
                    .strip_suffix(entry.as_str())
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
    }
}

fn normalize_host(host: &str) -> String {
    let host = host.trim().trim_end_matches('.');
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    host.to_ascii_lowercase()
}

/// Read a variable spelled in upper and lower case. Both are always read;
/// set to different values they are refused rather than one silently
/// winning. Empty counts as unset.
fn either_case(
    get: &impl Fn(&str) -> Option<String>,
    upper: &'static str,
    lower: &'static str,
) -> Result<Option<(&'static str, String)>, ConfigError> {
    match (non_empty(get, upper), non_empty(get, lower)) {
        (Some(a), Some(b)) if a != b => Err(ConfigError {
            variable: lower,
            expected: "unset, or the same value as the upper-case variable",
        }),
        (Some(value), _) => Ok(Some((upper, value))),
        (None, Some(value)) => Ok(Some((lower, value))),
        (None, None) => Ok(None),
    }
}

fn proxy(get: &impl Fn(&str) -> Option<String>) -> Result<Option<FetchProxy>, ConfigError> {
    let url = either_case(get, "HTTPS_PROXY", "https_proxy")?;
    let no_proxy = either_case(get, "NO_PROXY", "no_proxy")?;
    let Some((variable, url)) = url else {
        return Ok(None);
    };
    let (host, port, userinfo) = parse_proxy_url(&url).ok_or(ConfigError {
        variable,
        expected: PROXY_EXPECTED,
    })?;
    let no_proxy = no_proxy
        .map(|(_, list)| {
            list.split(',')
                .map(|entry| {
                    let entry = entry.trim();
                    let entry = entry
                        .strip_prefix("*.")
                        .or_else(|| entry.strip_prefix('.'))
                        .unwrap_or(entry);
                    normalize_host(entry)
                })
                .filter(|entry| !entry.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(FetchProxy {
        host,
        port,
        userinfo,
        no_proxy,
    }))
}

/// Whether `userinfo` is RFC 3986 user information whose percent-encoding
/// decodes to UTF-8, as the HTTP client requires.
fn valid_userinfo(userinfo: &str) -> bool {
    let bytes = userinfo.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            let Some(hex) = userinfo.get(index + 1..index + 3) else {
                return false;
            };
            let Ok(value) = u8::from_str_radix(hex, 16) else {
                return false;
            };
            decoded.push(value);
            index += 3;
        } else if byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:".contains(&byte) {
            decoded.push(byte);
            index += 1;
        } else {
            return false;
        }
    }
    String::from_utf8(decoded).is_ok()
}

/// Parse `[http://][user[:password]@]host[:port][/]`. The port defaults to
/// 80. Any other scheme, a path, query or fragment is refused.
fn parse_proxy_url(value: &str) -> Option<(String, u16, Option<String>)> {
    let value = value.trim();
    let rest = match value.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
        Some(_) => return None,
        None => value,
    };
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty()
        || authority
            .chars()
            .any(|c| matches!(c, '/' | '?' | '#' | '\\') || c.is_whitespace() || c.is_control())
    {
        return None;
    }
    let (userinfo, host_port) = match authority.rsplit_once('@') {
        Some((userinfo, host_port)) => (Some(userinfo), host_port),
        None => (None, authority),
    };
    if let Some(userinfo) = userinfo
        && (userinfo.is_empty() || !valid_userinfo(userinfo))
    {
        return None;
    }
    let parsed: axum::http::uri::Authority = host_port.parse().ok()?;
    let host = parsed.host();
    if host.is_empty() {
        return None;
    }
    // The client's parser maps an out-of-range port to "no port", so read
    // the port text here.
    let port = match host_port.strip_prefix(host)? {
        "" => 80,
        text => match text.strip_prefix(':')?.parse::<u16>().ok()? {
            0 => return None,
            port => port,
        },
    };
    Some((host.to_owned(), port, userinfo.map(str::to_owned)))
}

/// Default bound on one file's download.
pub const DEFAULT_FILE_TIMEOUT: Duration = Duration::from_secs(3_600);

/// Whether `name` can be a bundle name: 1 to 128 ASCII letters, digits,
/// `.`, `-` or `_`, starting with a letter or digit. It is one path segment
/// in both download URLs and in the cache directory.
pub fn valid_bundle_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(b))
}

/// Whether `revision` is a full commit id: exactly 40 lowercase hex
/// characters. Branch names such as `main`, tags and short ids are refused.
pub fn valid_revision(revision: &str) -> bool {
    revision.len() == 40
        && revision
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
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
    /// Permit an unauthenticated listener on an isolated development network.
    pub allow_unauthenticated_non_loopback: bool,
    /// Read the pool's service bearer from this mounted file at startup.
    pub service_bearer_file: Option<PathBuf>,
    pub embedding: StageBackend,
    pub reranking: StageBackend,
    /// Dimension of the reference embedding backend.
    pub reference_dimensions: usize,
    /// Artifacts of a model-backed embedding stage.
    pub embedding_artifacts: Option<StageArtifacts>,
    /// Artifacts of a model-backed reranking stage.
    pub reranking_artifacts: Option<StageArtifacts>,
    /// Bundle fetched for the embedding stage at startup. Its artifacts are
    /// then `<cache>/<bundle>`.
    pub embedding_bundle: Option<StageBundle>,
    /// Bundle fetched for the reranking stage at startup.
    pub reranking_bundle: Option<StageBundle>,
    /// Present exactly when a stage has a bundle.
    pub model_cache: Option<ModelCache>,
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
            allow_unauthenticated_non_loopback: false,
            service_bearer_file: None,
            embedding: StageBackend::Disabled,
            reranking: StageBackend::Disabled,
            reference_dimensions: 1024,
            embedding_artifacts: None,
            reranking_artifacts: None,
            embedding_bundle: None,
            reranking_bundle: None,
            model_cache: None,
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

const REVISION_EXPECTED: &str =
    "the pinned commit: 40 lowercase hex characters, not a branch or tag";

fn non_empty(get: &impl Fn(&str) -> Option<String>, variable: &str) -> Option<String> {
    get(variable)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Read a stage's bundle name and pinned revision. Both or neither must be
/// set, only for a model-backed stage and only in a build that can fetch.
fn bundle(
    get: &impl Fn(&str) -> Option<String>,
    name_variable: &'static str,
    revision_variable: &'static str,
    backend: StageBackend,
) -> Result<Option<StageBundle>, ConfigError> {
    let name = non_empty(get, name_variable);
    let revision = non_empty(get, revision_variable);
    let error = |variable, expected| Err(ConfigError { variable, expected });
    match (name, revision) {
        (None, None) => Ok(None),
        (None, Some(_)) => error(name_variable, "a bundle name with the revision"),
        (Some(_), _) if !cfg!(feature = "fetch") => {
            error(name_variable, "unset: this build has no fetch feature")
        }
        (Some(_), _) if !is_model_backed(backend) => {
            error(name_variable, "unset unless the stage uses a model backend")
        }
        (Some(name), _) if !valid_bundle_name(&name) => error(
            name_variable,
            "a bundle name of letters, digits, '.', '-' or '_'",
        ),
        (Some(_), None) => error(revision_variable, REVISION_EXPECTED),
        (Some(_), Some(revision)) if !valid_revision(&revision) => {
            error(revision_variable, REVISION_EXPECTED)
        }
        (Some(bundle), Some(revision)) => Ok(Some(StageBundle { bundle, revision })),
    }
}

/// Read the shared cache settings, which exist exactly when a stage has a
/// bundle. The proxy variables are read only then, so a deployment without
/// a bundle is never affected by them.
fn model_cache(
    get: &impl Fn(&str) -> Option<String>,
    any_bundle: bool,
) -> Result<Option<ModelCache>, ConfigError> {
    let directory = non_empty(get, "LENS_MODEL_CACHE_DIR");
    let timeout_set = get("LENS_MODEL_FETCH_TIMEOUT_SECS").is_some();
    let seconds = number(
        get,
        "LENS_MODEL_FETCH_TIMEOUT_SECS",
        1,
        86_400,
        "seconds from 1 to 86400",
        DEFAULT_FILE_TIMEOUT.as_secs() as usize,
    )?;
    match (any_bundle, directory) {
        (false, None) if !timeout_set => Ok(None),
        (false, None) => Err(ConfigError {
            variable: "LENS_MODEL_FETCH_TIMEOUT_SECS",
            expected: "unset unless a stage sets a bundle",
        }),
        (false, Some(_)) => Err(ConfigError {
            variable: "LENS_MODEL_CACHE_DIR",
            expected: "unset unless a stage sets a bundle",
        }),
        (true, None) => Err(ConfigError {
            variable: "LENS_MODEL_CACHE_DIR",
            expected: "a writable cache directory for the stage bundles",
        }),
        (true, Some(directory)) => Ok(Some(ModelCache {
            directory: PathBuf::from(directory),
            file_timeout: Duration::from_secs(seconds as u64),
            proxy: proxy(get)?,
        })),
    }
}

/// Read a stage's artifact directory and manifest digest. Both or neither
/// must be set, and both are required when `required` is true. With a
/// bundle the directory is `<cache>/<bundle>` and must not also be set.
fn artifacts(
    get: &impl Fn(&str) -> Option<String>,
    directory_variable: &'static str,
    digest_variable: &'static str,
    required: bool,
    bundle_directory: Option<PathBuf>,
) -> Result<Option<StageArtifacts>, ConfigError> {
    let directory = get(directory_variable).filter(|value| !value.trim().is_empty());
    let digest = get(digest_variable);
    let directory = match (directory, bundle_directory) {
        (Some(_), Some(_)) => {
            return Err(ConfigError {
                variable: directory_variable,
                expected: "unset when the stage's bundle is set",
            });
        }
        (None, Some(bundle)) => {
            let digest = digest.ok_or(ConfigError {
                variable: digest_variable,
                expected: DIGEST_EXPECTED,
            })?;
            return Ok(Some(StageArtifacts {
                directory: bundle,
                manifest_sha256: parse_sha256(digest.trim()).ok_or(ConfigError {
                    variable: digest_variable,
                    expected: DIGEST_EXPECTED,
                })?,
            }));
        }
        (directory, None) => directory,
    };
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
        let embedding_bundle = bundle(
            &get,
            "LENS_EMBEDDING_BUNDLE",
            "LENS_EMBEDDING_BUNDLE_REVISION",
            embedding,
        )?;
        let reranking_bundle = bundle(
            &get,
            "LENS_RERANKING_BUNDLE",
            "LENS_RERANKING_BUNDLE_REVISION",
            reranking,
        )?;
        let model_cache = model_cache(
            &get,
            embedding_bundle.is_some() || reranking_bundle.is_some(),
        )?;
        let bundle_directory = |bundle: &Option<StageBundle>| {
            Option::zip(bundle.as_ref(), model_cache.as_ref())
                .map(|(bundle, cache)| cache.directory.join(&bundle.bundle))
        };
        Ok(Self {
            // The range check above keeps the port within u16.
            listen: SocketAddr::new(host, port as u16),
            allow_unauthenticated_non_loopback: allow,
            service_bearer_file: get("LENS_SERVICE_BEARER_FILE").map(PathBuf::from),
            embedding,
            reranking,
            embedding_artifacts: artifacts(
                &get,
                "LENS_EMBEDDING_ARTIFACTS",
                "LENS_EMBEDDING_MANIFEST_SHA256",
                is_model_backed(embedding),
                bundle_directory(&embedding_bundle),
            )?,
            reranking_artifacts: artifacts(
                &get,
                "LENS_RERANKING_ARTIFACTS",
                "LENS_RERANKING_MANIFEST_SHA256",
                is_model_backed(reranking),
                bundle_directory(&reranking_bundle),
            )?,
            embedding_bundle,
            reranking_bundle,
            model_cache,
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

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn without_fetch_settings_there_is_nothing_to_fetch() {
        let config = lookup(&[]).unwrap();
        assert_eq!(config.embedding_bundle, None);
        assert_eq!(config.reranking_bundle, None);
        assert_eq!(config.model_cache, None);
        // Cache settings without a bundle are refused rather than ignored.
        let error = lookup(&[("LENS_MODEL_CACHE_DIR", "/var/cache/lens")]).unwrap_err();
        assert_eq!(error.variable, "LENS_MODEL_CACHE_DIR");
        let error = lookup(&[("LENS_MODEL_FETCH_TIMEOUT_SECS", "60")]).unwrap_err();
        assert_eq!(error.variable, "LENS_MODEL_FETCH_TIMEOUT_SECS");
        let error = lookup(&[("LENS_RERANKING_BUNDLE_REVISION", COMMIT)]).unwrap_err();
        assert_eq!(error.variable, "LENS_RERANKING_BUNDLE");
    }

    #[cfg(not(feature = "fetch"))]
    #[test]
    fn refuses_a_bundle_in_a_build_without_the_fetcher() {
        let error = lookup(&[
            ("LENS_EMBEDDING_BUNDLE", "qwen3-embedding-0.6b-f32-1024d"),
            ("LENS_EMBEDDING_BUNDLE_REVISION", COMMIT),
        ])
        .unwrap_err();
        assert_eq!(error.variable, "LENS_EMBEDDING_BUNDLE");
        assert_eq!(error.expected, "unset: this build has no fetch feature");
    }

    #[cfg(all(feature = "fetch", feature = "qwen3"))]
    #[test]
    fn reads_a_pinned_bundle_into_the_cache() {
        let digest = "0f".repeat(32);
        let base = [
            ("LENS_EMBEDDING_BACKEND", "qwen3"),
            ("LENS_EMBEDDING_BUNDLE", "qwen3-embedding-0.6b-f32-1024d"),
            ("LENS_EMBEDDING_BUNDLE_REVISION", COMMIT),
            ("LENS_EMBEDDING_MANIFEST_SHA256", digest.as_str()),
            ("LENS_MODEL_CACHE_DIR", "/var/cache/lens"),
        ];
        let config = lookup(&base).unwrap();
        assert_eq!(
            config.embedding_bundle,
            Some(StageBundle {
                bundle: "qwen3-embedding-0.6b-f32-1024d".to_owned(),
                revision: COMMIT.to_owned(),
            })
        );
        let cache = config.model_cache.unwrap();
        assert_eq!(cache.directory, PathBuf::from("/var/cache/lens"));
        assert_eq!(cache.file_timeout, DEFAULT_FILE_TIMEOUT);
        assert_eq!(
            config.embedding_artifacts.unwrap().directory,
            PathBuf::from("/var/cache/lens").join("qwen3-embedding-0.6b-f32-1024d")
        );
        let with = |extra: &[(&'static str, &'static str)]| {
            let mut pairs = base.to_vec();
            pairs.retain(|(name, _)| extra.iter().all(|(other, _)| other != name));
            pairs.extend_from_slice(extra);
            lookup(&pairs)
        };
        for (name, value, variable) in [
            (
                "LENS_EMBEDDING_BUNDLE_REVISION",
                "main",
                "LENS_EMBEDDING_BUNDLE_REVISION",
            ),
            (
                "LENS_EMBEDDING_BUNDLE_REVISION",
                "v1.0",
                "LENS_EMBEDDING_BUNDLE_REVISION",
            ),
            (
                "LENS_EMBEDDING_BUNDLE_REVISION",
                "0123456",
                "LENS_EMBEDDING_BUNDLE_REVISION",
            ),
            (
                "LENS_EMBEDDING_BUNDLE",
                "../escape",
                "LENS_EMBEDDING_BUNDLE",
            ),
            ("LENS_EMBEDDING_BUNDLE", "models/x", "LENS_EMBEDDING_BUNDLE"),
            (
                "LENS_EMBEDDING_ARTIFACTS",
                "/srv/lens/embedding",
                "LENS_EMBEDDING_ARTIFACTS",
            ),
            (
                "LENS_EMBEDDING_BACKEND",
                "reference",
                "LENS_EMBEDDING_BUNDLE",
            ),
            ("LENS_MODEL_CACHE_DIR", " ", "LENS_MODEL_CACHE_DIR"),
            (
                "LENS_MODEL_FETCH_TIMEOUT_SECS",
                "0",
                "LENS_MODEL_FETCH_TIMEOUT_SECS",
            ),
        ] {
            let error = with(&[(name, value)]).unwrap_err();
            assert_eq!(error.variable, variable, "{name}={value}");
            if value.len() >= 4 {
                assert!(!error.to_string().contains(value), "{name}");
            }
        }
        let mut no_digest = base.to_vec();
        no_digest.retain(|(name, _)| *name != "LENS_EMBEDDING_MANIFEST_SHA256");
        assert_eq!(
            lookup(&no_digest).unwrap_err().variable,
            "LENS_EMBEDDING_MANIFEST_SHA256"
        );
        let timed = with(&[("LENS_MODEL_FETCH_TIMEOUT_SECS", "600")]).unwrap();
        assert_eq!(
            timed.model_cache.unwrap().file_timeout,
            Duration::from_secs(600)
        );
    }

    fn proxy_lookup(pairs: &[(&str, &str)]) -> Result<Option<FetchProxy>, ConfigError> {
        FetchProxy::from_lookup(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn reads_an_http_connect_proxy_from_the_standard_variables() {
        assert_eq!(proxy_lookup(&[]).unwrap(), None);
        // NO_PROXY alone configures no proxy.
        assert_eq!(proxy_lookup(&[("NO_PROXY", "*")]).unwrap(), None);
        for (pairs, host, port) in [
            (
                &[("HTTPS_PROXY", "http://proxy.example:3128")][..],
                "proxy.example",
                3128,
            ),
            (
                &[("https_proxy", "HTTP://proxy.example/")][..],
                "proxy.example",
                80,
            ),
            (&[("HTTPS_PROXY", "10.0.0.7:8080")][..], "10.0.0.7", 8080),
            (&[("HTTPS_PROXY", "http://[::1]:3128")][..], "[::1]", 3128),
            (
                &[
                    ("HTTPS_PROXY", "http://proxy.example:3128"),
                    ("https_proxy", "http://proxy.example:3128"),
                ][..],
                "proxy.example",
                3128,
            ),
        ] {
            let proxy = proxy_lookup(pairs).unwrap().unwrap();
            assert_eq!((proxy.host(), proxy.port()), (host, port), "{pairs:?}");
            assert!(!proxy.has_credentials());
        }
        let proxy = proxy_lookup(&[("HTTPS_PROXY", "http://proxy.example:3128")])
            .unwrap()
            .unwrap();
        assert_eq!(proxy.client_url(), "http://proxy.example:3128");
    }

    #[test]
    fn refuses_unsupported_proxy_urls_without_echoing_them() {
        for value in [
            "https://proxy.example:3128",
            "socks5://proxy.example:1080",
            "socks5h://proxy.example:1080",
            "socks://proxy.example",
            "ftp://proxy.example",
            "http://proxy.example:3128/path",
            "http://proxy.example:3128?x=1",
            "http://proxy.example:3128#x",
            "http://proxy.example:0",
            "http://proxy.example:99999",
            "http://proxy.example:",
            "http://",
            "http://user:p@ss@proxy.example:3128",
            "http://user:%zz@proxy.example:3128",
            "http://user:%ff@proxy.example:3128",
            "http://@proxy.example:3128",
            "http://proxy example:3128",
        ] {
            let error = proxy_lookup(&[("HTTPS_PROXY", value)]).unwrap_err();
            assert_eq!(error.variable, "HTTPS_PROXY", "{value}");
            assert_eq!(error.expected, PROXY_EXPECTED, "{value}");
            // The message names the scheme it expects; any longer value
            // never appears in it.
            if value != "http://" {
                assert!(!error.to_string().contains(value), "{value}");
            }
        }
        // The two spellings must agree.
        let error = proxy_lookup(&[
            ("HTTPS_PROXY", "http://a.example:3128"),
            ("https_proxy", "http://b.example:3128"),
        ])
        .unwrap_err();
        assert_eq!(error.variable, "https_proxy");
        assert!(!error.to_string().contains("example"));
        let error = proxy_lookup(&[
            ("HTTPS_PROXY", "http://a.example:3128"),
            ("NO_PROXY", "a"),
            ("no_proxy", "b"),
        ])
        .unwrap_err();
        assert_eq!(error.variable, "no_proxy");
    }

    #[test]
    fn proxy_credentials_never_appear_in_debug_output_or_errors() {
        let proxy = proxy_lookup(&[(
            "HTTPS_PROXY",
            "http://lens-user:s%40cret-pw@proxy.example:3128",
        )])
        .unwrap()
        .unwrap();
        assert!(proxy.has_credentials());
        // The client gets the credentials, still percent-encoded.
        assert_eq!(
            proxy.client_url(),
            "http://lens-user:s%40cret-pw@proxy.example:3128"
        );
        let cache = ModelCache {
            directory: PathBuf::from("/var/cache/lens"),
            file_timeout: DEFAULT_FILE_TIMEOUT,
            proxy: Some(proxy.clone()),
        };
        let config = Config {
            model_cache: Some(cache),
            ..Config::default()
        };
        for text in [
            format!("{proxy:?}"),
            format!("{config:?}"),
            format!("{config:#?}"),
        ] {
            assert!(text.contains("<redacted>"), "{text}");
            for secret in ["lens-user", "s%40cret-pw", "s@cret-pw", "cret"] {
                assert!(!text.contains(secret), "{text}");
            }
        }
        let error = proxy_lookup(&[("HTTPS_PROXY", "socks5://lens-user:s3cret@proxy.example")])
            .unwrap_err();
        assert!(!error.to_string().contains("s3cret"));
        assert!(!format!("{error:?}").contains("s3cret"));
    }

    #[test]
    fn no_proxy_matches_names_suffixes_and_addresses() {
        let proxy = proxy_lookup(&[
            ("HTTPS_PROXY", "http://proxy.example:3128"),
            (
                "no_proxy",
                " internal.example, .HF.CO ,*.corp.example,10.0.0.0/8,127.0.0.1,[::1],,",
            ),
        ])
        .unwrap()
        .unwrap();
        for host in [
            "internal.example",
            "a.internal.example",
            "cdn-lfs.hf.co",
            "hf.co",
            "x.corp.example",
            "corp.example",
            "127.0.0.1",
            "[::1]",
            "CDN-LFS.HF.CO",
        ] {
            assert!(proxy.bypasses(host), "{host}");
        }
        for host in [
            "huggingface.co",
            "github.com",
            "notinternal.example",
            "internal.example.net",
            "10.1.2.3",
            "127.0.0.2",
        ] {
            assert!(!proxy.bypasses(host), "{host}");
        }
        let all = proxy_lookup(&[
            ("HTTPS_PROXY", "http://proxy.example:3128"),
            ("NO_PROXY", "*"),
        ])
        .unwrap()
        .unwrap();
        assert!(all.bypasses("huggingface.co"));
        let none = proxy_lookup(&[("HTTPS_PROXY", "http://proxy.example:3128")])
            .unwrap()
            .unwrap();
        assert!(!none.bypasses("huggingface.co"));
    }

    #[test]
    fn proxy_variables_are_ignored_without_a_bundle() {
        // A reference deployment in an environment with an unsupported proxy
        // still starts: the variables are read only with a bundle.
        let config = lookup(&[
            ("HTTPS_PROXY", "socks5://proxy.example:1080"),
            ("https_proxy", "http://other.example:3128"),
        ])
        .unwrap();
        assert_eq!(config, Config::default());
    }

    #[cfg(all(feature = "fetch", feature = "qwen3"))]
    #[test]
    fn a_bundle_reads_the_proxy_settings() {
        let digest = "0f".repeat(32);
        let base = [
            ("LENS_EMBEDDING_BACKEND", "qwen3"),
            ("LENS_EMBEDDING_BUNDLE", "qwen3-embedding-0.6b-f32-1024d"),
            ("LENS_EMBEDDING_BUNDLE_REVISION", COMMIT),
            ("LENS_EMBEDDING_MANIFEST_SHA256", digest.as_str()),
            ("LENS_MODEL_CACHE_DIR", "/var/cache/lens"),
        ];
        assert_eq!(lookup(&base).unwrap().model_cache.unwrap().proxy, None);
        let mut proxied = base.to_vec();
        proxied.push(("HTTPS_PROXY", "http://proxy.example:3128"));
        proxied.push(("NO_PROXY", "internal.example"));
        let proxy = lookup(&proxied)
            .unwrap()
            .model_cache
            .unwrap()
            .proxy
            .unwrap();
        assert_eq!((proxy.host(), proxy.port()), ("proxy.example", 3128));
        assert!(proxy.bypasses("internal.example"));
        let mut refused = base.to_vec();
        refused.push(("HTTPS_PROXY", "socks5://proxy.example:1080"));
        assert_eq!(lookup(&refused).unwrap_err().variable, "HTTPS_PROXY");
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
