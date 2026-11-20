// SPDX-License-Identifier: Apache-2.0

//! Download a pinned model bundle from the two fixed Munarium locations.
//!
//! Present only with the `fetch` cargo feature. A bundle is pinned by its
//! name, a Hugging Face repository commit and the SHA-256 of its
//! `lens-manifest.json`. The fetcher downloads the manifest first and checks
//! its digest and schema with the same code that loads a stage, then
//! downloads only the files the manifest lists. Each file streams into a
//! temporary file beside its final name, capped at the listed size, and is
//! renamed into place only after its size and SHA-256 match. The finished
//! directory is checked again from disk.
//!
//! The two locations are compile-time constants and are tried in a fixed
//! order: Hugging Face, then the GitHub Releases backup only if Hugging Face
//! fails. Neither the base URLs nor the order are configurable, and no other
//! host is fetched from: redirects are followed only over HTTPS to the hosts
//! each location is known to redirect to. Only a `#[cfg(test)]` build can
//! point the fetcher at a local test server.
//!
//! An egress proxy from the standard `HTTPS_PROXY` and `NO_PROXY` settings
//! (read by configuration, see [`FetchProxy`]) is a transport route only.
//! Each request to a host the proxy applies to goes through an HTTP
//! `CONNECT` tunnel with TLS end to end to the target host inside it, so
//! certificate validation and the redirect allowlist apply exactly as on a
//! direct connection. The proxy is chosen per request after the allowlist
//! check, never names a host, and its own answers are never followed as
//! redirects. A non-HTTPS target is never sent through the proxy.
//!
//! Diagnostics carry a fixed category and never a URL, path or value.

use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::config::AutoHeaderValue;
use ureq::http::Uri;

use crate::config::{FetchProxy, parse_sha256, valid_bundle_name, valid_revision};
use crate::manifest::{self, MANIFEST_FILE, MAX_MANIFEST_BYTES};

/// The Hugging Face model repository holding every Lens bundle. Fixed.
pub const HUGGING_FACE_BASE: &str = "https://huggingface.co/iokaio/munarium-lens-models";
/// Release downloads of this repository, the backup location. Fixed.
pub const GITHUB_RELEASES_BASE: &str = "https://github.com/iokaio/munarium-lens/releases/download/";

/// Hosts a Hugging Face download may reach: the repository host, which
/// serves small files through a same-host `/api/resolve-cache/` redirect,
/// and the CDN hosts it redirects large (Xet or LFS) files to.
pub const HUGGING_FACE_HOSTS: &[&str] = &[
    "huggingface.co",
    "cas-bridge.xethub.hf.co",
    "cdn-lfs.hf.co",
    "cdn-lfs-us-1.hf.co",
    "cdn-lfs-eu-1.hf.co",
];
/// Hosts a GitHub release download may reach: `github.com` and the
/// release-asset hosts it redirects to.
pub const GITHUB_HOSTS: &[&str] = &[
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];
/// Most redirects followed for one file.
pub const MAX_REDIRECTS: usize = 5;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const PARTIAL_PREFIX: &str = ".lens-partial-";

/// A download location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `huggingface.co/iokaio/munarium-lens-models`, at the pinned commit.
    HuggingFace,
    /// The `models/<bundle>` release of `iokaio/munarium-lens`.
    GitHubReleases,
}

/// The fixed order sources are tried in. Not configurable: the GitHub
/// backup is used only when Hugging Face fails.
pub const SOURCE_ORDER: [Source; 2] = [Source::HuggingFace, Source::GitHubReleases];

impl Source {
    /// Fixed label for diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::HuggingFace => "huggingface",
            Self::GitHubReleases => "github",
        }
    }

    /// The fixed base URL.
    pub fn base(self) -> &'static str {
        match self {
            Self::HuggingFace => HUGGING_FACE_BASE,
            Self::GitHubReleases => GITHUB_RELEASES_BASE,
        }
    }

    /// The hosts this source may reach, including through redirects.
    pub fn hosts(self) -> &'static [&'static str] {
        match self {
            Self::HuggingFace => HUGGING_FACE_HOSTS,
            Self::GitHubReleases => GITHUB_HOSTS,
        }
    }

    /// Base URL for a request. Only a test build can substitute it.
    fn request_base(self) -> String {
        #[cfg(test)]
        if let Some(base) = tests::override_base(self) {
            return base;
        }
        self.base().to_owned()
    }
}

/// Percent-encode one path segment, leaving only RFC 3986 unreserved bytes.
fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The URL of one bundle file at `source`, or `None` when the source cannot
/// address it: an invalid bundle name or revision, or a nested path for the
/// GitHub backup, whose release assets are flat.
///
/// - Hugging Face: `<base>/resolve/<revision>/<bundle>/<path>`.
/// - GitHub: `<base>models/<bundle>/<file>`. The tag `models/<bundle>` keeps
///   its `/` unencoded, as GitHub's own `browser_download_url` does.
pub fn file_url(source: Source, revision: &str, bundle: &str, path: &str) -> Option<String> {
    if !valid_revision(revision) || !valid_bundle_name(bundle) || path.is_empty() {
        return None;
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return None;
    }
    let encoded: Vec<String> = segments
        .iter()
        .map(|segment| encode_segment(segment))
        .collect();
    let base = source.request_base();
    match source {
        Source::HuggingFace => Some(format!(
            "{}/resolve/{revision}/{bundle}/{}",
            base.trim_end_matches('/'),
            encoded.join("/")
        )),
        Source::GitHubReleases if encoded.len() == 1 => Some(format!(
            "{}/models/{bundle}/{}",
            base.trim_end_matches('/'),
            encoded[0]
        )),
        Source::GitHubReleases => None,
    }
}

/// Check that `url` may be requested for `source`: HTTPS on the default
/// port, no user information, and a host in the source's fixed list.
fn allowed(source: Source, url: &str) -> Option<Uri> {
    let uri: Uri = url.parse().ok()?;
    let authority = uri.authority()?;
    if authority.as_str().contains('@') {
        return None;
    }
    let host = authority.host();
    #[cfg(test)]
    if tests::test_endpoint(&uri) {
        return Some(uri);
    }
    let fixed_host = source.hosts().contains(&host);
    let https = uri.scheme_str() == Some("https");
    let default_port = matches!(uri.port_u16(), None | Some(443));
    (fixed_host && https && default_port).then_some(uri)
}

/// Resolve a `Location` header against the current URL. Absolute URLs and
/// absolute paths are accepted; the result is checked again by [`allowed`].
fn resolve_location(current: &Uri, location: &str) -> Option<String> {
    if location.starts_with('/') && !location.starts_with("//") {
        let scheme = current.scheme_str()?;
        let authority = current.authority()?;
        Some(format!("{scheme}://{authority}{location}"))
    } else if location.contains("://") {
        Some(location.to_owned())
    } else {
        None
    }
}

/// Content-free reason a fetch failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// The bundle name, revision or manifest digest is not a valid pin.
    InvalidPin,
    /// The cache directory is inside a Munarium Lens source checkout.
    CacheInSourceTree,
    /// A directory or file could not be created, written or renamed.
    WriteFailed,
    /// Connection, TLS, timeout or an unexpected HTTP status.
    SourceUnavailable,
    /// The source answered 404.
    NotFound,
    /// A redirect left HTTPS or the source's fixed hosts.
    RedirectRefused,
    /// More than [`MAX_REDIRECTS`] redirects.
    TooManyRedirects,
    /// The HTTP client refused the configured proxy URL.
    InvalidProxy,
    /// The proxy did not open a tunnel: it refused, asked for credentials,
    /// answered with a redirect or did not answer `CONNECT`.
    ProxyFailed,
    /// A request through the proxy would not have been HTTPS.
    ProxyTargetNotHttps,
    /// A response was longer than the manifest allows.
    ResponseTooLarge,
    /// The source cannot address the file (a nested path on GitHub).
    UnsupportedPath,
    /// The downloaded manifest is not the pinned one.
    ManifestDigestMismatch,
    /// The manifest is pinned but invalid; carries the manifest category.
    ManifestRejected(&'static str),
    /// The manifest's served name is not the bundle name.
    BundleNameMismatch,
    /// A downloaded file's size or SHA-256 differs from the manifest.
    FileDigestMismatch,
    /// The final check of the directory failed; carries the category.
    VerifyFailed(&'static str),
}

impl FetchError {
    pub fn category(self) -> &'static str {
        match self {
            Self::InvalidPin => "invalid_pin",
            Self::CacheInSourceTree => "cache_in_source_tree",
            Self::WriteFailed => "write_failed",
            Self::SourceUnavailable => "source_unavailable",
            Self::NotFound => "not_found",
            Self::RedirectRefused => "redirect_refused",
            Self::TooManyRedirects => "too_many_redirects",
            Self::InvalidProxy => "invalid_proxy",
            Self::ProxyFailed => "proxy_failed",
            Self::ProxyTargetNotHttps => "proxy_target_not_https",
            Self::ResponseTooLarge => "response_too_large",
            Self::UnsupportedPath => "unsupported_path",
            Self::ManifestDigestMismatch => "manifest_digest_mismatch",
            Self::ManifestRejected(category) | Self::VerifyFailed(category) => category,
            Self::BundleNameMismatch => "bundle_name_mismatch",
            Self::FileDigestMismatch => "file_digest_mismatch",
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.category())
    }
}

impl std::error::Error for FetchError {}

/// A pinned bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub bundle: String,
    pub revision: String,
    pub manifest_sha256: [u8; 32],
}

impl Pin {
    /// Validate a pin. The digest is the manifest's lowercase hex SHA-256.
    pub fn new(bundle: &str, revision: &str, manifest_sha256: &str) -> Result<Self, FetchError> {
        if !valid_bundle_name(bundle) || !valid_revision(revision) {
            return Err(FetchError::InvalidPin);
        }
        Ok(Self {
            bundle: bundle.to_owned(),
            revision: revision.to_owned(),
            manifest_sha256: parse_sha256(manifest_sha256).ok_or(FetchError::InvalidPin)?,
        })
    }
}

/// A verified bundle directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// `<cache>/<bundle>`.
    pub directory: PathBuf,
    pub manifest_sha256: [u8; 32],
    /// Listed files downloaded by this call.
    pub fetched: usize,
    /// Listed files already present and verified, so not downloaded.
    pub reused: usize,
}

/// Whether `path` is inside a Munarium Lens source checkout: an ancestor
/// holding `.git` and this package's `Cargo.toml`. The cache must not be,
/// so downloaded weights never land in a working tree or build context.
fn in_source_tree(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor.join(".git").exists()
            && fs::read_to_string(ancestor.join("Cargo.toml"))
                .is_ok_and(|manifest| manifest.contains("name = \"munarium-lens\""))
    })
}

/// Remove temporary files left in `directory` by an interrupted fetch.
fn remove_partials(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let is_partial = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(PARTIAL_PREFIX));
        if is_partial && entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// A temporary file beside its final name, removed unless committed.
struct Partial {
    path: PathBuf,
    file: Option<fs::File>,
}

impl Partial {
    fn create(directory: &Path) -> Result<Self, FetchError> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = directory.join(format!(
            "{PARTIAL_PREFIX}{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = fs::File::create_new(&path).map_err(|_| FetchError::WriteFailed)?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }

    /// Flush to disk, then atomically rename over `target`.
    fn commit(mut self, target: &Path) -> Result<(), FetchError> {
        let file = self.file.take().ok_or(FetchError::WriteFailed)?;
        file.sync_all().map_err(|_| FetchError::WriteFailed)?;
        drop(file);
        fs::rename(&self.path, target).map_err(|_| FetchError::WriteFailed)?;
        // Renamed: nothing left to remove.
        self.path = PathBuf::new();
        #[cfg(unix)]
        if let Some(parent) = target.parent() {
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| FetchError::WriteFailed)?;
        }
        Ok(())
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Stream at most `size` bytes from `reader` into a temporary file beside
/// `target`, require exactly `size` bytes with digest `sha256`, then rename
/// it into place. On any failure the temporary file is removed and `target`
/// is untouched.
fn write_verified(
    target: &Path,
    reader: impl Read,
    size: u64,
    sha256: &[u8; 32],
    deadline: Instant,
) -> Result<(), FetchError> {
    use sha2::Digest;
    let directory = target.parent().ok_or(FetchError::WriteFailed)?;
    let mut partial = Partial::create(directory)?;
    let mut reader = reader.take(size.saturating_add(1));
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| FetchError::SourceUnavailable)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > size {
            return Err(FetchError::ResponseTooLarge);
        }
        if Instant::now() > deadline {
            return Err(FetchError::SourceUnavailable);
        }
        hasher.update(&buffer[..read]);
        partial
            .file
            .as_mut()
            .ok_or(FetchError::WriteFailed)?
            .write_all(&buffer[..read])
            .map_err(|_| FetchError::WriteFailed)?;
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if total != size || &digest != sha256 {
        return Err(FetchError::FileDigestMismatch);
    }
    partial.commit(target)
}

/// The HTTP client. Its own proxy setting is always `None`, so it never
/// uses ambient proxy settings; [`Fetcher::route`] picks the proxy per
/// request.
fn agent() -> Agent {
    let config = Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .https_only(!cfg!(test))
        .proxy(None)
        .user_agent("munarium-lens")
        .accept_encoding(AutoHeaderValue::None)
        .timeout_resolve(Some(CONNECT_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RESPONSE_TIMEOUT))
        .timeout_per_read(Some(READ_TIMEOUT))
        .build();
    client(config)
}

#[cfg(not(test))]
fn client(config: ureq::config::Config) -> Agent {
    Agent::new_with_config(config)
}

/// A test build resolves no name but 127.0.0.1, so no test, and no mutation
/// of this module under test, can reach a real host.
#[cfg(test)]
fn client(config: ureq::config::Config) -> Agent {
    Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::new(),
        tests::LoopbackOnly::default(),
    )
}

/// Write one content-free diagnostic line.
fn diagnostic(line: &str) {
    #[cfg(test)]
    tests::note_diagnostic(line);
    eprintln!("{line}");
}

/// The configured proxy: its settings and the HTTP client's form of it.
struct Route<'a> {
    settings: &'a FetchProxy,
    client: ureq::Proxy,
}

struct Fetcher<'a> {
    agent: Agent,
    file_timeout: Duration,
    proxy: Option<Route<'a>>,
}

impl<'a> Fetcher<'a> {
    fn new(file_timeout: Duration, proxy: Option<&'a FetchProxy>) -> Result<Self, FetchError> {
        let proxy = proxy
            .map(|settings| {
                // Credentials stay inside the client; its error is dropped
                // unprinted.
                ureq::Proxy::new(&settings.client_url())
                    .map(|client| Route { settings, client })
                    .map_err(|_| FetchError::InvalidProxy)
            })
            .transpose()?;
        Ok(Self {
            agent: agent(),
            file_timeout,
            proxy,
        })
    }

    /// The proxy for a request to `uri`, already checked by [`allowed`]:
    /// none without a proxy or when `NO_PROXY` matches the host, and never
    /// for a target that is not HTTPS.
    fn route(&self, uri: &Uri) -> Result<Option<ureq::Proxy>, FetchError> {
        let Some(route) = &self.proxy else {
            return Ok(None);
        };
        if route.settings.bypasses(uri.host().unwrap_or_default()) {
            return Ok(None);
        }
        #[cfg(test)]
        if tests::test_endpoint(uri) {
            return Ok(Some(route.client.clone()));
        }
        if uri.scheme_str() != Some("https") {
            return Err(FetchError::ProxyTargetNotHttps);
        }
        Ok(Some(route.client.clone()))
    }

    /// GET `url`, following only allowed redirects, until a 200 response.
    fn get(
        &self,
        source: Source,
        url: String,
        deadline: Instant,
    ) -> Result<ureq::http::Response<ureq::Body>, FetchError> {
        let mut url = url;
        for _ in 0..=MAX_REDIRECTS {
            let uri = allowed(source, &url).ok_or(FetchError::RedirectRefused)?;
            let proxy = self.route(&uri)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(FetchError::SourceUnavailable);
            }
            #[cfg(test)]
            tests::note_request();
            let response = self
                .agent
                .get(&url)
                .config()
                .proxy(proxy)
                .timeout_global(Some(remaining))
                .build()
                .call()
                .map_err(|error| match error {
                    ureq::Error::ConnectProxyFailed(_) => FetchError::ProxyFailed,
                    _ => FetchError::SourceUnavailable,
                })?;
            match response.status().as_u16() {
                200 => return Ok(response),
                301 | 302 | 303 | 307 | 308 => {
                    let location = response
                        .headers()
                        .get("location")
                        .and_then(|value| value.to_str().ok())
                        .ok_or(FetchError::RedirectRefused)?;
                    url = resolve_location(&uri, location).ok_or(FetchError::RedirectRefused)?;
                }
                404 => return Err(FetchError::NotFound),
                _ => return Err(FetchError::SourceUnavailable),
            }
        }
        Err(FetchError::TooManyRedirects)
    }

    /// Download the manifest from `source` and require the pinned digest.
    fn manifest(&self, source: Source, pin: &Pin) -> Result<Vec<u8>, FetchError> {
        let url = file_url(source, &pin.revision, &pin.bundle, MANIFEST_FILE)
            .ok_or(FetchError::UnsupportedPath)?;
        let deadline = Instant::now() + self.file_timeout;
        let response = self.get(source, url, deadline)?;
        if response
            .body()
            .content_length()
            .is_some_and(|length| length > MAX_MANIFEST_BYTES)
        {
            return Err(FetchError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| FetchError::SourceUnavailable)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(FetchError::ResponseTooLarge);
        }
        if manifest::sha256(&bytes) != pin.manifest_sha256 {
            return Err(FetchError::ManifestDigestMismatch);
        }
        Ok(bytes)
    }

    /// Download one listed file from `source` into `target`.
    fn file(
        &self,
        source: Source,
        pin: &Pin,
        file: &manifest::ListedFile,
        target: &Path,
    ) -> Result<(), FetchError> {
        let url = file_url(source, &pin.revision, &pin.bundle, &file.path)
            .ok_or(FetchError::UnsupportedPath)?;
        let deadline = Instant::now() + self.file_timeout;
        let response = self.get(source, url, deadline)?;
        if response
            .body()
            .content_length()
            .is_some_and(|length| length > file.size)
        {
            return Err(FetchError::ResponseTooLarge);
        }
        let reader = response.into_body().into_reader();
        write_verified(target, reader, file.size, &file.sha256, deadline)
    }
}

/// Try each source in [`SOURCE_ORDER`] until one succeeds.
fn first_success<T>(
    what: &str,
    mut attempt: impl FnMut(Source) -> Result<T, FetchError>,
) -> Result<T, FetchError> {
    let mut last = FetchError::SourceUnavailable;
    for source in SOURCE_ORDER {
        match attempt(source) {
            Ok(value) => return Ok(value),
            Err(error) => {
                diagnostic(&format!(
                    "Lens fetch: {what} from {} failed: {error}",
                    source.label()
                ));
                last = error;
            }
        }
    }
    Err(last)
}

fn file_matches(path: &Path, file: &manifest::ListedFile) -> bool {
    manifest::hash_regular(path, file.size) == Some((file.size, file.sha256))
}

/// Make `<cache>/<bundle>` hold the pinned bundle, downloading only what is
/// missing or wrong, and verify it. A bundle already complete on disk makes
/// no connection. With `proxy`, requests to hosts it applies to go through
/// its `CONNECT` tunnel.
pub fn fetch_bundle(
    cache: &Path,
    pin: &Pin,
    file_timeout: Duration,
    proxy: Option<&FetchProxy>,
) -> Result<Outcome, FetchError> {
    if !valid_bundle_name(&pin.bundle) || !valid_revision(&pin.revision) {
        return Err(FetchError::InvalidPin);
    }
    let absolute = std::path::absolute(cache).map_err(|_| FetchError::WriteFailed)?;
    if in_source_tree(&absolute) {
        return Err(FetchError::CacheInSourceTree);
    }
    let directory = absolute.join(&pin.bundle);
    fs::create_dir_all(&directory).map_err(|_| FetchError::WriteFailed)?;
    let canonical = fs::canonicalize(&directory).map_err(|_| FetchError::WriteFailed)?;
    if in_source_tree(&canonical) {
        return Err(FetchError::CacheInSourceTree);
    }
    remove_partials(&directory);

    let fetcher = Fetcher::new(file_timeout, proxy)?;
    let manifest_path = directory.join(MANIFEST_FILE);
    let local = manifest::read_regular(&manifest_path, MAX_MANIFEST_BYTES)
        .filter(|bytes| manifest::sha256(bytes) == pin.manifest_sha256);
    let manifest_present = local.is_some();
    let bytes = match local {
        Some(bytes) => bytes,
        None => first_success("manifest", |source| fetcher.manifest(source, pin))?,
    };
    let listing = manifest::parse(&bytes, &pin.manifest_sha256)
        .map_err(|error| FetchError::ManifestRejected(error.category()))?;
    if listing.served_model() != pin.bundle {
        return Err(FetchError::BundleNameMismatch);
    }

    let (mut fetched, mut reused) = (0, 0);
    for file in &listing.files {
        let target = directory.join(&file.path);
        if file_matches(&target, file) {
            reused += 1;
            continue;
        }
        let parent = target.parent().ok_or(FetchError::WriteFailed)?;
        fs::create_dir_all(parent).map_err(|_| FetchError::WriteFailed)?;
        remove_partials(parent);
        first_success("file", |source| fetcher.file(source, pin, file, &target))?;
        fetched += 1;
    }
    if !manifest_present {
        let deadline = Instant::now() + file_timeout;
        write_verified(
            &manifest_path,
            bytes.as_slice(),
            bytes.len() as u64,
            &pin.manifest_sha256,
            deadline,
        )?;
    }
    let served = manifest::verify_files(&directory, &pin.manifest_sha256)
        .map_err(|error| FetchError::VerifyFailed(error.category()))?;
    if served != pin.bundle {
        return Err(FetchError::BundleNameMismatch);
    }
    Ok(Outcome {
        directory,
        manifest_sha256: pin.manifest_sha256,
        fetched,
        reused,
    })
}

/// Arguments of `munarium-lens fetch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchCommand {
    pub pin: Pin,
    pub cache_dir: PathBuf,
    pub file_timeout: Duration,
}

/// Usage of the `fetch` subcommand. There is no source, URL or order option.
pub const USAGE: &str = "usage: munarium-lens fetch --bundle <name> --revision <40-hex commit> \
--manifest-sha256 <hex> --cache-dir <directory> [--timeout-secs <1-86400>]";

/// Parse the arguments after `fetch`. Errors name the problem, never a value.
pub fn parse_command(args: &[String]) -> Result<FetchCommand, &'static str> {
    let mut bundle = None;
    let mut revision = None;
    let mut digest = None;
    let mut cache_dir = None;
    let mut timeout = None;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let slot = match flag.as_str() {
            "--bundle" => &mut bundle,
            "--revision" => &mut revision,
            "--manifest-sha256" => &mut digest,
            "--cache-dir" => &mut cache_dir,
            "--timeout-secs" => &mut timeout,
            _ => return Err("unknown argument"),
        };
        let value = args.next().ok_or("missing value")?;
        if slot.replace(value.clone()).is_some() {
            return Err("repeated argument");
        }
    }
    let bundle = bundle.ok_or("--bundle is required")?;
    let revision = revision.ok_or("--revision is required")?;
    let digest = digest.ok_or("--manifest-sha256 is required")?;
    let cache_dir = cache_dir.ok_or("--cache-dir is required")?;
    if !valid_bundle_name(&bundle) {
        return Err("--bundle must be letters, digits, '.', '-' or '_'");
    }
    if !valid_revision(&revision) {
        return Err("--revision must be a 40-character lowercase hex commit, not a branch or tag");
    }
    let pin = Pin::new(&bundle, &revision, &digest)
        .map_err(|_| "--manifest-sha256 must be a lowercase hex SHA-256")?;
    let file_timeout = match timeout {
        None => crate::config::DEFAULT_FILE_TIMEOUT,
        Some(text) => match text.parse::<u64>() {
            Ok(seconds @ 1..=86_400) => Duration::from_secs(seconds),
            _ => return Err("--timeout-secs must be from 1 to 86400"),
        },
    };
    Ok(FetchCommand {
        pin,
        cache_dir: PathBuf::from(cache_dir),
        file_timeout,
    })
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Run `munarium-lens fetch`, through `proxy` when the standard proxy
/// settings name one. Prints the verified manifest digest on standard
/// output. Returns the process exit code: 0 verified, 1 failed, 2 usage
/// error.
pub fn run_command(args: &[String], proxy: Option<&FetchProxy>) -> i32 {
    let command = match parse_command(args) {
        Ok(command) => command,
        Err(problem) => {
            eprintln!("{problem}\n{USAGE}");
            return 2;
        }
    };
    match fetch_bundle(
        &command.cache_dir,
        &command.pin,
        command.file_timeout,
        proxy,
    ) {
        Ok(outcome) => {
            eprintln!(
                "Lens fetch: bundle verified ({} downloaded, {} already present)",
                outcome.fetched, outcome.reused
            );
            println!("{}", hex(&outcome.manifest_sha256));
            0
        }
        Err(error) => {
            eprintln!("Lens fetch failed: {error}");
            1
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::manifest::tests::{Scratch, hex as hex_digest};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    // ---- Test-only endpoint injection -------------------------------------

    struct Endpoints {
        hugging_face: String,
        github: String,
    }

    thread_local! {
        static ENDPOINTS: RefCell<Option<Endpoints>> = const { RefCell::new(None) };
        static REQUESTS: Cell<usize> = const { Cell::new(0) };
        static DIAGNOSTICS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn note_diagnostic(line: &str) {
        DIAGNOSTICS.with_borrow_mut(|lines| lines.push(line.to_owned()));
    }

    /// Diagnostic lines this thread's fetcher has written.
    fn diagnostics() -> Vec<String> {
        DIAGNOSTICS.with_borrow(Clone::clone)
    }

    /// The test client's resolver: 127.0.0.1 only. Any other name fails
    /// before a connection, so a test can never reach a real host, even
    /// when a mutation makes the fetcher skip the proxy.
    #[derive(Debug, Default)]
    pub(crate) struct LoopbackOnly(ureq::unversioned::resolver::DefaultResolver);

    impl ureq::unversioned::resolver::Resolver for LoopbackOnly {
        fn resolve(
            &self,
            uri: &Uri,
            config: &ureq::config::Config,
            timeout: ureq::unversioned::transport::NextTimeout,
        ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
            if uri.host() != Some("127.0.0.1") {
                return Err(ureq::Error::HostNotFound);
            }
            self.0.resolve(uri, config, timeout)
        }
    }

    pub(crate) fn override_base(source: Source) -> Option<String> {
        ENDPOINTS.with_borrow(|endpoints| {
            endpoints.as_ref().map(|endpoints| match source {
                Source::HuggingFace => endpoints.hugging_face.clone(),
                Source::GitHubReleases => endpoints.github.clone(),
            })
        })
    }

    /// With test endpoints set, plain HTTP to 127.0.0.1 is allowed. No other
    /// host is added: a redirect to `localhost` is still refused.
    pub(crate) fn test_endpoint(uri: &Uri) -> bool {
        ENDPOINTS.with_borrow(|endpoints| endpoints.is_some())
            && uri.scheme_str() == Some("http")
            && uri.host() == Some("127.0.0.1")
    }

    pub(crate) fn note_request() {
        REQUESTS.set(REQUESTS.get() + 1);
    }

    /// Requests this thread's fetcher has sent.
    pub(crate) fn requests() -> usize {
        REQUESTS.get()
    }

    /// Point this thread's fetcher at local servers until dropped.
    pub(crate) struct UseEndpoints;

    impl UseEndpoints {
        pub(crate) fn new(hugging_face: String, github: String) -> Self {
            ENDPOINTS.set(Some(Endpoints {
                hugging_face,
                github,
            }));
            Self
        }
    }

    impl Drop for UseEndpoints {
        fn drop(&mut self) {
            ENDPOINTS.set(None);
        }
    }

    // ---- A minimal local HTTP server --------------------------------------

    #[derive(Clone)]
    pub(crate) enum Reply {
        Body(Vec<u8>),
        /// A body without `Content-Length`, ended by closing the connection.
        Unsized(Vec<u8>),
        /// A `Content-Length` larger than the body actually sent.
        Declared(u64, Vec<u8>),
        Redirect(String),
        Status(u16),
    }

    pub(crate) struct TestServer {
        pub(crate) address: SocketAddr,
        routes: Arc<Mutex<HashMap<String, Reply>>>,
        hits: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
    }

    impl TestServer {
        pub(crate) fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let routes: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
            let hits: Arc<Mutex<Vec<String>>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let (thread_routes, thread_hits, thread_stop) =
                (routes.clone(), hits.clone(), stop.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if thread_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        serve_one(stream, &thread_routes, &thread_hits);
                    }
                }
            });
            Self {
                address,
                routes,
                hits,
                stop,
            }
        }

        pub(crate) fn origin(&self) -> String {
            format!("http://{}", self.address)
        }

        pub(crate) fn route(&self, path: &str, reply: Reply) {
            self.routes.lock().unwrap().insert(path.to_owned(), reply);
        }

        pub(crate) fn hits(&self) -> Vec<String> {
            self.hits.lock().unwrap().clone()
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(self.address);
        }
    }

    fn serve_one(
        stream: TcpStream,
        routes: &Mutex<HashMap<String, Reply>>,
        hits: &Mutex<Vec<String>>,
    ) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if line == "\r\n" => break,
                Ok(_) => {}
            }
        }
        let target = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        if target.is_empty() {
            return;
        }
        hits.lock().unwrap().push(target.clone());
        let path = target.split('?').next().unwrap_or_default();
        let reply = routes
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or(Reply::Status(404));
        let mut stream = reader.into_inner();
        let head = |status: &str, extra: &str| {
            format!("HTTP/1.1 {status}\r\n{extra}Connection: close\r\n\r\n")
        };
        let _ = match reply {
            Reply::Body(body) => stream
                .write_all(
                    head("200 OK", &format!("Content-Length: {}\r\n", body.len())).as_bytes(),
                )
                .and_then(|()| stream.write_all(&body)),
            Reply::Unsized(body) => stream
                .write_all(head("200 OK", "").as_bytes())
                .and_then(|()| stream.write_all(&body)),
            Reply::Declared(length, body) => stream
                .write_all(head("200 OK", &format!("Content-Length: {length}\r\n")).as_bytes())
                .and_then(|()| stream.write_all(&body)),
            Reply::Redirect(location) => stream.write_all(
                head(
                    "307 Temporary Redirect",
                    &format!("Location: {location}\r\nContent-Length: 0\r\n"),
                )
                .as_bytes(),
            ),
            Reply::Status(code) => stream
                .write_all(head(&format!("{code} Status"), "Content-Length: 0\r\n").as_bytes()),
        };
        let _ = stream.flush();
    }

    /// An address nothing listens on.
    fn closed_origin() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{address}")
    }

    // ---- A minimal local CONNECT proxy ----------------------------------------

    #[derive(Clone)]
    enum ProxyMode {
        /// Open the tunnel and relay it, but only to a 127.0.0.1 port.
        Tunnel,
        /// Open the tunnel, then answer the request inside it with a
        /// redirect to this location instead of relaying it.
        Redirect(String),
        /// Answer `CONNECT` itself with this status line and headers.
        Refuse(String),
        /// Open the tunnel, record the first bytes the client sends (its TLS
        /// ClientHello), answer them with a plaintext redirect and close.
        /// Nothing is relayed, so no real host is contacted.
        Sniff,
    }

    #[derive(Default)]
    struct ProxyLog {
        /// `CONNECT` targets, in order.
        connects: Vec<String>,
        /// The `Proxy-Authorization` header of each `CONNECT`.
        authorizations: Vec<Option<String>>,
        /// The first bytes sent inside each sniffed tunnel.
        first_bytes: Vec<Vec<u8>>,
    }

    struct TestProxy {
        address: SocketAddr,
        log: Arc<Mutex<ProxyLog>>,
        stop: Arc<AtomicBool>,
    }

    impl TestProxy {
        fn start(mode: ProxyMode) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let log: Arc<Mutex<ProxyLog>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let (thread_log, thread_stop) = (log.clone(), stop.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if thread_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        let (log, mode) = (thread_log.clone(), mode.clone());
                        std::thread::spawn(move || proxy_one(stream, &mode, &log));
                    }
                }
            });
            Self { address, log, stop }
        }

        /// `HTTPS_PROXY` pointing at this proxy, optionally with
        /// credentials, and `NO_PROXY`.
        fn settings(&self, userinfo: &str, no_proxy: &str) -> FetchProxy {
            let url = format!("http://{userinfo}{}", self.address);
            FetchProxy::from_lookup(|name| match name {
                "HTTPS_PROXY" => Some(url.clone()),
                "NO_PROXY" => Some(no_proxy.to_owned()),
                _ => None,
            })
            .unwrap()
            .unwrap()
        }

        fn connects(&self) -> Vec<String> {
            self.log.lock().unwrap().connects.clone()
        }

        fn authorizations(&self) -> Vec<Option<String>> {
            self.log.lock().unwrap().authorizations.clone()
        }

        fn first_bytes(&self) -> Vec<Vec<u8>> {
            self.log.lock().unwrap().first_bytes.clone()
        }
    }

    impl Drop for TestProxy {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(self.address);
        }
    }

    /// Read a request head; return its request line and headers.
    fn read_head(reader: &mut BufReader<TcpStream>) -> Option<(String, Vec<String>)> {
        let mut request_line = String::new();
        reader.read_line(&mut request_line).ok()?;
        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return None,
                Ok(_) if line == "\r\n" => break,
                Ok(_) => headers.push(line.trim_end().to_owned()),
            }
        }
        Some((request_line, headers))
    }

    fn proxy_one(stream: TcpStream, mode: &ProxyMode, log: &Mutex<ProxyLog>) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(stream);
        let Some((request_line, headers)) = read_head(&mut reader) else {
            return;
        };
        let mut parts = request_line.split_whitespace();
        if parts.next() != Some("CONNECT") {
            return;
        }
        let target = parts.next().unwrap_or_default().to_owned();
        let authorization = headers.iter().find_map(|header| {
            let (name, value) = header.split_once(':')?;
            name.eq_ignore_ascii_case("proxy-authorization")
                .then(|| value.trim().to_owned())
        });
        {
            let mut log = log.lock().unwrap();
            log.connects.push(target.clone());
            log.authorizations.push(authorization);
        }
        let mut client = reader.into_inner();
        const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
        match mode {
            ProxyMode::Refuse(head) => {
                let _ = client.write_all(format!("HTTP/1.1 {head}\r\n\r\n").as_bytes());
            }
            ProxyMode::Sniff => {
                let _ = client.write_all(ESTABLISHED);
                let mut bytes = vec![0u8; 16 * 1024];
                let mut length = 0;
                // A ClientHello record: 5-byte header, then its length.
                while length < 5
                    || length < 5 + usize::from(u16::from_be_bytes([bytes[3], bytes[4]]))
                {
                    match client.read(&mut bytes[length..]) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => length += read,
                    }
                    if length == bytes.len() {
                        break;
                    }
                }
                bytes.truncate(length);
                log.lock().unwrap().first_bytes.push(bytes);
                let _ = client.write_all(
                    b"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:9/x\r\n\r\n",
                );
            }
            ProxyMode::Redirect(location) => {
                let _ = client.write_all(ESTABLISHED);
                let mut reader = BufReader::new(client);
                if read_head(&mut reader).is_some() {
                    let _ = reader.into_inner().write_all(
                        format!(
                            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\n\
                             Content-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    );
                }
            }
            ProxyMode::Tunnel => {
                let upstream = target
                    .strip_prefix("127.0.0.1:")
                    .and_then(|port| TcpStream::connect(("127.0.0.1", port.parse().ok()?)).ok());
                let Some(upstream) = upstream else {
                    let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n");
                    return;
                };
                let _ = client.write_all(ESTABLISHED);
                let (mut up_read, mut up_write) = (upstream.try_clone().unwrap(), upstream);
                let (mut down_read, mut down_write) = (client.try_clone().unwrap(), client);
                let relay = std::thread::spawn(move || {
                    let _ = std::io::copy(&mut down_read, &mut up_write);
                    let _ = up_write.shutdown(std::net::Shutdown::Write);
                });
                let _ = std::io::copy(&mut up_read, &mut down_write);
                let _ = down_write.shutdown(std::net::Shutdown::Both);
                let _ = relay.join();
            }
        }
    }

    // ---- A synthetic bundle ------------------------------------------------

    pub(crate) const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
    pub(crate) const BUNDLE: &str = "fixture-embedding-4d";

    pub(crate) struct Bundle {
        pub(crate) files: Vec<(String, Vec<u8>)>,
        pub(crate) manifest: Vec<u8>,
    }

    impl Bundle {
        pub(crate) fn new(files: &[(&str, &[u8])]) -> Self {
            let entries: Vec<String> = files
                .iter()
                .map(|(path, bytes)| {
                    format!(
                        r#"{{"path":"{path}","size":{},"sha256":"{}"}}"#,
                        bytes.len(),
                        hex_digest(&manifest::sha256(bytes))
                    )
                })
                .collect();
            let manifest = format!(
                r#"{{"format":"{}","stage":"embedding","backend":"qwen3","served_model":"{BUNDLE}","precision":"f32","dimensions":1024,"upstream":{{"repository":"fixture/model","revision":"0000"}},"files":[{}]}}"#,
                manifest::FORMAT,
                entries.join(",")
            )
            .into_bytes();
            Self {
                files: files
                    .iter()
                    .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
                    .collect(),
                manifest,
            }
        }

        pub(crate) fn pin(&self) -> Pin {
            Pin {
                bundle: BUNDLE.to_owned(),
                revision: REVISION.to_owned(),
                manifest_sha256: manifest::sha256(&self.manifest),
            }
        }
    }

    pub(crate) fn hf_path(file: &str) -> String {
        format!("/hf/resolve/{REVISION}/{BUNDLE}/{file}")
    }

    pub(crate) fn gh_path(file: &str) -> String {
        format!("/gh/models/{BUNDLE}/{file}")
    }

    /// Serve `bundle` from `server` under the Hugging Face layout.
    pub(crate) fn serve_hf(server: &TestServer, bundle: &Bundle) {
        server.route(
            &hf_path(MANIFEST_FILE),
            Reply::Body(bundle.manifest.clone()),
        );
        for (path, bytes) in &bundle.files {
            server.route(&hf_path(path), Reply::Body(bytes.clone()));
        }
    }

    /// Serve `bundle` from `server` under the GitHub release layout.
    fn serve_gh(server: &TestServer, bundle: &Bundle) {
        server.route(
            &gh_path(MANIFEST_FILE),
            Reply::Body(bundle.manifest.clone()),
        );
        for (path, bytes) in &bundle.files {
            server.route(&gh_path(path), Reply::Body(bytes.clone()));
        }
    }

    fn sample() -> Bundle {
        Bundle::new(&[
            ("config.json", b"{\"synthetic\":true}"),
            ("tokenizer.json", b"synthetic tokenizer"),
            ("model.safetensors", b"synthetic weights, not a model"),
        ])
    }

    fn endpoints(hugging_face: &str, github: &str) -> UseEndpoints {
        UseEndpoints::new(format!("{hugging_face}/hf"), format!("{github}/gh/"))
    }

    fn fetch(cache: &Scratch, pin: &Pin) -> Result<Outcome, FetchError> {
        fetch_bundle(&cache.0, pin, Duration::from_secs(30), None)
    }

    fn fetch_via(cache: &Scratch, pin: &Pin, proxy: &FetchProxy) -> Result<Outcome, FetchError> {
        fetch_bundle(&cache.0, pin, Duration::from_secs(30), Some(proxy))
    }

    fn partials(directory: &Path) -> Vec<String> {
        fs::read_dir(directory)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| name.starts_with(PARTIAL_PREFIX))
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---- Unit tests ----------------------------------------------------------

    #[test]
    fn builds_fixed_urls_for_each_source() {
        let revision = "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3";
        let bundle = "qwen3-embedding-0.6b-f32-1024d";
        assert_eq!(
            file_url(Source::HuggingFace, revision, bundle, "model.safetensors").unwrap(),
            "https://huggingface.co/iokaio/munarium-lens-models/resolve/97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3/qwen3-embedding-0.6b-f32-1024d/model.safetensors"
        );
        // The tag `models/<bundle>` keeps its slash, as GitHub's own
        // browser_download_url does for a tag containing `/`.
        assert_eq!(
            file_url(
                Source::GitHubReleases,
                revision,
                bundle,
                "lens-manifest.json"
            )
            .unwrap(),
            "https://github.com/iokaio/munarium-lens/releases/download/models/qwen3-embedding-0.6b-f32-1024d/lens-manifest.json"
        );
        // Nested paths: encoded per segment on Hugging Face; release assets
        // are flat, so GitHub cannot address them.
        assert_eq!(
            file_url(Source::HuggingFace, revision, bundle, "sub dir/a+b.json").unwrap(),
            format!("{HUGGING_FACE_BASE}/resolve/{revision}/{bundle}/sub%20dir/a%2Bb.json")
        );
        assert_eq!(
            file_url(Source::GitHubReleases, revision, bundle, "sub/a.json"),
            None
        );
        assert_eq!(
            file_url(Source::GitHubReleases, revision, bundle, "a?b#c.json").unwrap(),
            format!("{GITHUB_RELEASES_BASE}models/{bundle}/a%3Fb%23c.json")
        );
        for path in ["", "../x", "a//b", "./a"] {
            assert_eq!(
                file_url(Source::HuggingFace, revision, bundle, path),
                None,
                "{path}"
            );
        }
        assert_eq!(file_url(Source::HuggingFace, "main", bundle, "a"), None);
        assert_eq!(file_url(Source::HuggingFace, revision, "../x", "a"), None);
    }

    #[test]
    fn accepts_only_full_lowercase_commits() {
        assert!(valid_revision("97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3"));
        for revision in [
            "main",
            "v1.0",
            "refs/heads/main",
            "97b0c61",
            "97B0C614BE4D77EE51C0CEF4E5F07C00F9EB65B3",
            "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b30",
            "97b0c614be4d77ee51c0cef4e5f07c00f9eb65bg",
            "",
        ] {
            assert!(!valid_revision(revision), "{revision}");
        }
        assert!(valid_bundle_name("qwen3-embedding-0.6b-f32-1024d"));
        for name in ["", ".", "..", ".hidden", "a/b", "a\\b", "a b", "a%2Fb"] {
            assert!(!valid_bundle_name(name), "{name}");
        }
    }

    #[test]
    fn source_order_is_hugging_face_then_github() {
        assert_eq!(SOURCE_ORDER, [Source::HuggingFace, Source::GitHubReleases]);
    }

    #[test]
    fn allows_only_https_to_each_sources_fixed_hosts() {
        let ok = |source, url: &str| allowed(source, url).is_some();
        for host in HUGGING_FACE_HOSTS {
            assert!(
                ok(Source::HuggingFace, &format!("https://{host}/x?sig=1")),
                "{host}"
            );
            assert!(
                !ok(Source::GitHubReleases, &format!("https://{host}/x")),
                "{host}"
            );
        }
        for host in GITHUB_HOSTS {
            assert!(
                ok(Source::GitHubReleases, &format!("https://{host}/x")),
                "{host}"
            );
            assert!(
                !ok(Source::HuggingFace, &format!("https://{host}/x")),
                "{host}"
            );
        }
        for url in [
            "http://huggingface.co/x",
            "https://huggingface.co:8443/x",
            "https://user@huggingface.co/x",
            "https://huggingface.co.example.net/x",
            "https://evilhuggingface.co/x",
            "https://cdn-lfs.hf.co.example.net/x",
            "https://example.hf.co/x",
            "https://hf.co/x",
            "https://huggingface.co./x",
            "ftp://huggingface.co/x",
            "https://127.0.0.1/x",
            "/relative",
            "",
        ] {
            assert!(!ok(Source::HuggingFace, url), "{url}");
        }
        assert!(!ok(Source::GitHubReleases, "https://api.github.com/x"));
        assert!(!ok(
            Source::GitHubReleases,
            "https://raw.githubusercontent.com/x"
        ));
        // Plain HTTP to the test server is allowed only with test endpoints
        // set, and only for 127.0.0.1.
        assert!(!ok(Source::HuggingFace, "http://127.0.0.1:9/x"));
        let _endpoints = endpoints("http://127.0.0.1:9", "http://127.0.0.1:9");
        assert!(ok(Source::HuggingFace, "http://127.0.0.1:9/x"));
        assert!(!ok(Source::HuggingFace, "http://localhost:9/x"));
    }

    #[test]
    fn resolves_absolute_paths_and_urls_only() {
        let current: Uri = "https://huggingface.co/iokaio/x/resolve/c/b/f"
            .parse()
            .unwrap();
        assert_eq!(
            resolve_location(&current, "/api/resolve-cache/models/x?etag=1").unwrap(),
            "https://huggingface.co/api/resolve-cache/models/x?etag=1"
        );
        assert_eq!(
            resolve_location(&current, "https://cas-bridge.xethub.hf.co/y").unwrap(),
            "https://cas-bridge.xethub.hf.co/y"
        );
        assert_eq!(resolve_location(&current, "//evil.example/x"), None);
        assert_eq!(resolve_location(&current, "relative/x"), None);
    }

    #[test]
    fn the_agent_has_no_ambient_proxy_or_compression() {
        // The agent's own proxy is always None, whatever the environment
        // holds; only Fetcher::route chooses a proxy, per request.
        let agent = agent();
        assert!(agent.config().proxy().is_none());
        assert_eq!(agent.config().max_redirects(), 0);
        assert!(!agent.config().http_status_as_error());
    }

    #[test]
    fn parses_the_fetch_command_without_any_source_option() {
        let args =
            |list: &[&str]| -> Vec<String> { list.iter().map(|s| (*s).to_owned()).collect() };
        let digest = "ab".repeat(32);
        let good = [
            "--bundle",
            BUNDLE,
            "--revision",
            REVISION,
            "--manifest-sha256",
            &digest,
            "--cache-dir",
            "/var/cache/lens",
        ];
        let command = parse_command(&args(&good)).unwrap();
        assert_eq!(command.pin.bundle, BUNDLE);
        assert_eq!(command.pin.manifest_sha256, [0xab; 32]);
        assert_eq!(command.file_timeout, crate::config::DEFAULT_FILE_TIMEOUT);
        for extra in [
            ["--source", "github"],
            ["--sources", "github,huggingface"],
            ["--base-url", "https://mirror.example"],
            ["--mirror", "https://mirror.example"],
            ["--url", "https://mirror.example"],
            ["--endpoint", "https://mirror.example"],
        ] {
            let mut list = good.to_vec();
            list.extend(extra);
            assert_eq!(
                parse_command(&args(&list)),
                Err("unknown argument"),
                "{extra:?}"
            );
        }
        let mut branch = good.to_vec();
        branch[3] = "main";
        assert!(parse_command(&args(&branch)).is_err());
        let error = parse_command(&args(&good[..6])).unwrap_err();
        assert!(!error.contains(&digest));
    }

    // ---- Fetching against local servers -------------------------------------

    #[test]
    fn fetches_a_good_bundle_through_a_same_host_redirect() {
        let bundle = sample();
        let hf = TestServer::start();
        serve_hf(&hf, &bundle);
        // Like huggingface.co's /api/resolve-cache/ redirect for small files.
        hf.route(
            &hf_path("config.json"),
            Reply::Redirect("/hf/cache/config.json?etag=1".to_owned()),
        );
        hf.route(
            "/hf/cache/config.json",
            Reply::Body(bundle.files[0].1.clone()),
        );
        let gh = TestServer::start();
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        let outcome = fetch(&cache, &bundle.pin()).unwrap();
        assert_eq!((outcome.fetched, outcome.reused), (3, 0));
        assert_eq!(outcome.manifest_sha256, manifest::sha256(&bundle.manifest));
        let directory = cache.0.join(BUNDLE);
        assert_eq!(outcome.directory, std::path::absolute(&directory).unwrap());
        for (path, bytes) in &bundle.files {
            assert_eq!(&fs::read(directory.join(path)).unwrap(), bytes);
        }
        assert_eq!(
            fs::read(directory.join(MANIFEST_FILE)).unwrap(),
            bundle.manifest
        );
        assert!(partials(&directory).is_empty());
        // The manifest is fetched first; the backup is never contacted.
        assert_eq!(hf.hits()[0], hf_path(MANIFEST_FILE));
        assert!(gh.hits().is_empty());
    }

    #[test]
    fn a_wrong_manifest_digest_fetches_nothing_else() {
        let bundle = sample();
        let other = Bundle::new(&[("config.json", b"{}")]);
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &other);
        serve_gh(&gh, &other);
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        assert_eq!(
            fetch(&cache, &bundle.pin()).unwrap_err(),
            FetchError::ManifestDigestMismatch
        );
        assert_eq!(hf.hits(), vec![hf_path(MANIFEST_FILE)]);
        assert_eq!(gh.hits(), vec![gh_path(MANIFEST_FILE)]);
        let directory = cache.0.join(BUNDLE);
        assert!(!directory.join(MANIFEST_FILE).exists());
        assert!(!directory.join("config.json").exists());
    }

    #[test]
    fn a_corrupted_file_is_discarded_and_the_backup_is_tried() {
        let bundle = sample();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        serve_gh(&gh, &bundle);
        // Same size, different bytes.
        hf.route(
            &hf_path("model.safetensors"),
            Reply::Body(b"synthetic weights, not a modeL".to_vec()),
        );
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        let outcome = fetch(&cache, &bundle.pin()).unwrap();
        assert_eq!(outcome.fetched, 3);
        let directory = cache.0.join(BUNDLE);
        assert_eq!(
            fs::read(directory.join("model.safetensors")).unwrap(),
            bundle.files[2].1
        );
        assert!(partials(&directory).is_empty());
        assert_eq!(gh.hits(), vec![gh_path("model.safetensors")]);

        // With the backup also corrupt, nothing is left under the final name.
        let cache = Scratch::new();
        gh.route(
            &gh_path("model.safetensors"),
            Reply::Body(b"synthetic weights, not a modeL".to_vec()),
        );
        assert_eq!(
            fetch(&cache, &bundle.pin()).unwrap_err(),
            FetchError::FileDigestMismatch
        );
        let directory = cache.0.join(BUNDLE);
        assert!(!directory.join("model.safetensors").exists());
        assert!(!directory.join(MANIFEST_FILE).exists());
        assert!(partials(&directory).is_empty());
    }

    #[test]
    fn a_declared_oversized_length_is_refused_before_reading() {
        // The right bytes, but a Content-Length beyond the manifest size:
        // refused from the header alone, at both sources.
        let bundle = sample();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        serve_gh(&gh, &bundle);
        let bytes = bundle.files[1].1.clone();
        let declared = bytes.len() as u64 + 1_000_000;
        for server_path in [hf_path("tokenizer.json"), gh_path("tokenizer.json")] {
            let server = if server_path.starts_with("/hf") {
                &hf
            } else {
                &gh
            };
            server.route(&server_path, Reply::Declared(declared, bytes.clone()));
        }
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        assert_eq!(
            fetch(&cache, &bundle.pin()).unwrap_err(),
            FetchError::ResponseTooLarge
        );
        let directory = cache.0.join(BUNDLE);
        assert!(!directory.join("tokenizer.json").exists());
        assert!(partials(&directory).is_empty());
    }

    #[test]
    fn an_oversized_response_is_cut_off_at_the_manifest_size() {
        let bundle = sample();
        let mut long = bundle.files[1].1.clone();
        long.extend(std::iter::repeat_n(b'x', 1 << 20));
        for reply in [Reply::Body(long.clone()), Reply::Unsized(long)] {
            let (hf, gh) = (TestServer::start(), TestServer::start());
            serve_hf(&hf, &bundle);
            hf.route(&hf_path("tokenizer.json"), reply);
            let _endpoints = endpoints(&hf.origin(), &gh.origin());
            let cache = Scratch::new();
            assert_eq!(
                fetch(&cache, &bundle.pin()).unwrap_err(),
                FetchError::NotFound,
                "the backup has nothing, so its 404 is the last error"
            );
            let directory = cache.0.join(BUNDLE);
            assert!(!directory.join("tokenizer.json").exists());
            assert!(partials(&directory).is_empty());
        }
        // The cap itself: the writer never accepts a byte past the size.
        let cache = Scratch::new();
        let target = cache.0.join("file");
        let deadline = Instant::now() + Duration::from_secs(5);
        let bytes = b"0123456789".as_slice();
        assert_eq!(
            write_verified(&target, bytes, 9, &manifest::sha256(&bytes[..9]), deadline),
            Err(FetchError::ResponseTooLarge)
        );
        assert!(!target.exists());
        assert!(partials(&cache.0).is_empty());
        write_verified(
            &target,
            &bytes[..9],
            9,
            &manifest::sha256(&bytes[..9]),
            deadline,
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), &bytes[..9]);
    }

    #[test]
    fn refuses_a_redirect_to_another_host() {
        let bundle = sample();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        let port = hf.address.port();
        hf.route(
            &hf_path(MANIFEST_FILE),
            Reply::Redirect(format!("http://localhost:{port}/hf/elsewhere")),
        );
        hf.route("/hf/elsewhere", Reply::Body(bundle.manifest.clone()));
        gh.route(
            &gh_path(MANIFEST_FILE),
            Reply::Redirect("https://evil.example/lens-manifest.json".to_owned()),
        );
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        assert_eq!(
            fetch(&cache, &bundle.pin()).unwrap_err(),
            FetchError::RedirectRefused
        );
        // Neither redirect target was requested.
        assert_eq!(hf.hits(), vec![hf_path(MANIFEST_FILE)]);
        assert_eq!(gh.hits(), vec![gh_path(MANIFEST_FILE)]);
    }

    #[test]
    fn bounds_the_number_of_redirects() {
        let bundle = sample();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        hf.route(
            &hf_path(MANIFEST_FILE),
            Reply::Redirect("/hf/loop".to_owned()),
        );
        hf.route("/hf/loop", Reply::Redirect("/hf/loop".to_owned()));
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        assert_eq!(
            fetch(&cache, &bundle.pin()).unwrap_err(),
            FetchError::NotFound
        );
        assert_eq!(hf.hits().len(), MAX_REDIRECTS + 1);
    }

    #[test]
    fn uses_the_backup_only_when_hugging_face_fails() {
        let bundle = sample();
        // Hugging Face down: GitHub serves everything.
        let gh = TestServer::start();
        serve_gh(&gh, &bundle);
        let _endpoints = endpoints(&closed_origin(), &gh.origin());
        let cache = Scratch::new();
        assert_eq!(fetch(&cache, &bundle.pin()).unwrap().fetched, 3);
        assert_eq!(gh.hits()[0], gh_path(MANIFEST_FILE));
        assert_eq!(gh.hits().len(), 4);
        drop(_endpoints);

        // Hugging Face good: GitHub, also good, is never contacted.
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        serve_gh(&gh, &bundle);
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        fetch(&cache, &bundle.pin()).unwrap();
        assert_eq!(hf.hits().len(), 4);
        assert!(gh.hits().is_empty());
    }

    #[test]
    fn reuses_verified_files_and_replaces_wrong_ones() {
        let bundle = sample();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        let directory = cache.0.join(BUNDLE);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("model.safetensors"), &bundle.files[2].1).unwrap();
        fs::write(directory.join("tokenizer.json"), b"stale tokenizer").unwrap();
        fs::write(directory.join(".lens-partial-1-1"), b"left by a crash").unwrap();
        let outcome = fetch(&cache, &bundle.pin()).unwrap();
        assert_eq!((outcome.fetched, outcome.reused), (2, 1));
        assert!(!hf.hits().contains(&hf_path("model.safetensors")));
        assert_eq!(
            fs::read(directory.join("tokenizer.json")).unwrap(),
            bundle.files[1].1
        );
        assert!(partials(&directory).is_empty());

        // Complete on disk: no request at all.
        let before = requests();
        let outcome = fetch(&cache, &bundle.pin()).unwrap();
        assert_eq!((outcome.fetched, outcome.reused), (0, 3));
        assert_eq!(requests(), before);
    }

    #[test]
    fn refuses_a_bundle_whose_manifest_names_another_model() {
        let mut bundle = sample();
        bundle.manifest = String::from_utf8(bundle.manifest)
            .unwrap()
            .replace(BUNDLE, "another-bundle")
            .into_bytes();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        assert_eq!(
            fetch(&cache, &bundle.pin()).unwrap_err(),
            FetchError::BundleNameMismatch
        );
        assert_eq!(hf.hits(), vec![hf_path(MANIFEST_FILE)]);
    }

    #[test]
    fn refuses_a_pinned_manifest_listing_unsafe_paths() {
        let bundle = Bundle::new(&[("../escape.json", b"{}")]);
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let cache = Scratch::new();
        assert_eq!(
            fetch(&cache, &bundle.pin()).unwrap_err(),
            FetchError::ManifestRejected("unsafe_path")
        );
        assert_eq!(hf.hits(), vec![hf_path(MANIFEST_FILE)]);
        assert!(!cache.0.join("escape.json").exists());
    }

    #[test]
    fn refuses_a_cache_inside_a_lens_source_tree() {
        let tree = Scratch::new();
        fs::create_dir_all(tree.0.join(".git")).unwrap();
        fs::write(
            tree.0.join("Cargo.toml"),
            "[package]
name = \"munarium-lens\"
",
        )
        .unwrap();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let before = requests();
        let pin = sample().pin();
        assert_eq!(
            fetch_bundle(&tree.0.join("models"), &pin, Duration::from_secs(5), None).unwrap_err(),
            FetchError::CacheInSourceTree
        );
        assert_eq!(requests(), before);
        assert!(!tree.0.join("models").exists());
        // Another Git working tree, such as a dotfiles repository in a home
        // directory, is not a Lens source tree.
        let other = Scratch::new();
        fs::create_dir_all(other.0.join(".git")).unwrap();
        fs::write(
            other.0.join("Cargo.toml"),
            "[package]
name = \"other\"
",
        )
        .unwrap();
        assert!(!in_source_tree(&other.0.join("cache")));
        assert!(in_source_tree(&tree.0.join("cache")));
    }

    #[test]
    fn digests_are_checked_against_the_pin() {
        // Direct check of the digest comparisons the tests above rely on.
        let cache = Scratch::new();
        let target = cache.0.join("f");
        let deadline = Instant::now() + Duration::from_secs(5);
        assert_eq!(
            write_verified(&target, b"abc".as_slice(), 3, &[0; 32], deadline),
            Err(FetchError::FileDigestMismatch)
        );
        assert!(!target.exists());
        assert_eq!(
            write_verified(
                &target,
                b"ab".as_slice(),
                3,
                &manifest::sha256(b"ab"),
                deadline
            ),
            Err(FetchError::FileDigestMismatch)
        );
        assert!(partials(&cache.0).is_empty());
    }

    // ---- Fetching through an egress proxy ------------------------------------

    const CREDENTIALS: &str = "lens-user:s3cret-pw@";
    /// Base64 of `lens-user:s3cret-pw`.
    const BASIC: &str = "Basic bGVucy11c2VyOnMzY3JldC1wdw==";

    /// The TLS SNI host name in a ClientHello, if any.
    fn server_name(hello: &[u8]) -> Option<String> {
        // Record header (5), handshake header (4), version (2), random (32).
        let mut at = 5 + 4 + 2 + 32;
        let byte = |at: usize| hello.get(at).copied().map(usize::from);
        let pair = |at: usize| Some(byte(at)? << 8 | byte(at + 1)?);
        at += 1 + byte(at)?; // session id
        at += 2 + pair(at)?; // cipher suites
        at += 1 + byte(at)?; // compression methods
        let end = at + 2 + pair(at)?;
        at += 2;
        while at + 4 <= end {
            let (kind, length) = (pair(at)?, pair(at + 2)?);
            if kind == 0 {
                // server_name list (2), name type (1), name length (2).
                let name_length = pair(at + 4 + 3)?;
                let start = at + 4 + 5;
                return String::from_utf8(hello.get(start..start + name_length)?.to_vec()).ok();
            }
            at += 4 + length;
        }
        None
    }

    #[test]
    fn the_test_client_never_resolves_a_real_host() {
        let cache = Scratch::new();
        let before = requests();
        assert_eq!(
            fetch(&cache, &sample().pin()).unwrap_err(),
            FetchError::SourceUnavailable
        );
        // Both fixed sources were attempted and failed at name resolution.
        assert_eq!(requests(), before + 2);
    }

    #[test]
    fn fetches_through_a_connect_proxy_with_tls_to_the_fixed_hosts() {
        // No test endpoints: the production URLs. The proxy opens each
        // tunnel, records the TLS ClientHello and answers in plaintext as a
        // proxy cannot pass certificate validation. Nothing is relayed.
        let proxy = TestProxy::start(ProxyMode::Sniff);
        let cache = Scratch::new();
        assert_eq!(
            fetch_via(&cache, &sample().pin(), &proxy.settings("", "")).unwrap_err(),
            FetchError::SourceUnavailable
        );
        // The fixed hosts in the fixed order, on port 443; no other host,
        // and the proxy's plaintext redirect was not followed.
        assert_eq!(proxy.connects(), ["huggingface.co:443", "github.com:443"]);
        let hellos = proxy.first_bytes();
        assert_eq!(hellos.len(), 2);
        for (hello, host) in hellos.iter().zip(["huggingface.co", "github.com"]) {
            // A TLS handshake record: TLS runs end to end inside the tunnel,
            // addressed to the target host, not the proxy.
            assert_eq!(hello.first(), Some(&0x16), "{host}");
            assert_eq!(server_name(hello).as_deref(), Some(host));
        }
        assert!(!cache.0.join(BUNDLE).join(MANIFEST_FILE).exists());
    }

    #[test]
    fn a_whole_bundle_is_fetched_through_the_tunnel() {
        let bundle = sample();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let proxy = TestProxy::start(ProxyMode::Tunnel);
        let cache = Scratch::new();
        let outcome = fetch_via(&cache, &bundle.pin(), &proxy.settings("", "")).unwrap();
        assert_eq!(outcome.fetched, 3);
        // Every request went through a tunnel to the Hugging Face server,
        // manifest first, at the fixed-shape paths.
        let hf_target = hf.address.to_string();
        assert_eq!(proxy.connects(), vec![hf_target; 4]);
        assert_eq!(hf.hits()[0], hf_path(MANIFEST_FILE));
        assert_eq!(hf.hits().len(), 4);
        assert!(gh.hits().is_empty());
        assert_eq!(proxy.authorizations(), vec![None; 4]);
    }

    #[test]
    fn no_proxy_sends_matching_hosts_directly() {
        let bundle = sample();
        let (hf, gh) = (TestServer::start(), TestServer::start());
        serve_hf(&hf, &bundle);
        let _endpoints = endpoints(&hf.origin(), &gh.origin());
        let proxy = TestProxy::start(ProxyMode::Tunnel);
        let cache = Scratch::new();
        let bypass = proxy.settings("", "internal.example, 127.0.0.1");
        fetch_via(&cache, &bundle.pin(), &bypass).unwrap();
        assert!(proxy.connects().is_empty());
        assert_eq!(hf.hits().len(), 4);
        // An entry that does not match the host leaves the proxy in use.
        let cache = Scratch::new();
        let other = proxy.settings("", "internal.example,127.0.0.2");
        fetch_via(&cache, &bundle.pin(), &other).unwrap();
        assert_eq!(proxy.connects().len(), 4);
    }

    #[test]
    fn a_proxy_cannot_redirect_the_fetcher_off_the_allowlist() {
        let bundle = sample();
        for location in [
            "https://evil.example/lens-manifest.json".to_owned(),
            "http://huggingface.co/iokaio/munarium-lens-models/x".to_owned(),
            "http://localhost:9/hf/x".to_owned(),
        ] {
            // Inside the tunnel the proxy answers instead of relaying.
            let (hf, gh) = (TestServer::start(), TestServer::start());
            serve_hf(&hf, &bundle);
            serve_gh(&gh, &bundle);
            let _endpoints = endpoints(&hf.origin(), &gh.origin());
            let proxy = TestProxy::start(ProxyMode::Redirect(location.clone()));
            let cache = Scratch::new();
            assert_eq!(
                fetch_via(&cache, &bundle.pin(), &proxy.settings("", "")).unwrap_err(),
                FetchError::RedirectRefused,
                "{location}"
            );
            // One tunnel per fixed source, none to the redirect target.
            assert_eq!(
                proxy.connects(),
                [hf.address.to_string(), gh.address.to_string()],
                "{location}"
            );
            assert!(hf.hits().is_empty() && gh.hits().is_empty());
        }
        // Answering CONNECT itself with a redirect or an error opens no
        // tunnel and is never followed.
        for head in [
            "302 Found\r\nLocation: https://evil.example/x",
            "403 Forbidden",
            "502 Bad Gateway",
        ] {
            let (hf, gh) = (TestServer::start(), TestServer::start());
            serve_hf(&hf, &bundle);
            let _endpoints = endpoints(&hf.origin(), &gh.origin());
            let proxy = TestProxy::start(ProxyMode::Refuse(head.to_owned()));
            let cache = Scratch::new();
            assert_eq!(
                fetch_via(&cache, &bundle.pin(), &proxy.settings("", "")).unwrap_err(),
                FetchError::ProxyFailed,
                "{head}"
            );
            assert_eq!(proxy.connects().len(), 2, "{head}");
            assert!(
                proxy
                    .connects()
                    .iter()
                    .all(|target| target.starts_with("127.0.0.1:"))
            );
            assert!(hf.hits().is_empty());
        }
    }

    #[test]
    fn proxy_credentials_are_sent_to_the_proxy_and_never_logged() {
        let proxy = TestProxy::start(ProxyMode::Refuse(
            "407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"lens\""
                .to_owned(),
        ));
        let settings = proxy.settings(CREDENTIALS, "");
        let cache = Scratch::new();
        let error = fetch_via(&cache, &sample().pin(), &settings).unwrap_err();
        assert_eq!(error, FetchError::ProxyFailed);
        assert_eq!(proxy.connects(), ["huggingface.co:443", "github.com:443"]);
        assert_eq!(
            proxy.authorizations(),
            vec![Some(BASIC.to_owned()), Some(BASIC.to_owned())]
        );
        let lines = diagnostics();
        assert_eq!(
            lines[lines.len() - 2..],
            [
                "Lens fetch: manifest from huggingface failed: proxy_failed",
                "Lens fetch: manifest from github failed: proxy_failed",
            ]
        );
        let mut texts = lines;
        texts.push(error.to_string());
        texts.push(format!("{error:?}"));
        texts.push(format!("{settings:?}"));
        for text in &texts {
            for secret in ["lens-user", "s3cret", BASIC, &proxy.address.to_string()] {
                assert!(!text.contains(secret), "{text}");
            }
        }
    }

    #[test]
    fn routes_only_https_targets_through_the_proxy() {
        let proxy = TestProxy::start(ProxyMode::Tunnel);
        let settings = proxy.settings("", "cdn-lfs.hf.co");
        let fetcher = Fetcher::new(Duration::from_secs(5), Some(&settings)).unwrap();
        let route = |url: &str| {
            fetcher
                .route(&url.parse().unwrap())
                .map(|proxy| proxy.is_some())
        };
        assert_eq!(route("https://huggingface.co/x"), Ok(true));
        assert_eq!(route("https://cdn-lfs.hf.co/x"), Ok(false));
        assert_eq!(
            route("http://huggingface.co/x"),
            Err(FetchError::ProxyTargetNotHttps)
        );
        assert_eq!(
            route("http://127.0.0.1:9/x"),
            Err(FetchError::ProxyTargetNotHttps)
        );
        // Only with test endpoints set may the plain-HTTP test servers be
        // reached through the proxy.
        let _endpoints = endpoints("http://127.0.0.1:9", "http://127.0.0.1:9");
        assert_eq!(route("http://127.0.0.1:9/x"), Ok(true));
        assert_eq!(
            route("http://localhost:9/x"),
            Err(FetchError::ProxyTargetNotHttps)
        );
        let direct = Fetcher::new(Duration::from_secs(5), None).unwrap();
        assert_eq!(
            direct.route(&"https://huggingface.co/x".parse().unwrap()),
            Ok(None)
        );
    }
}
