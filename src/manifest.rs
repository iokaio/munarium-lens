// SPDX-License-Identifier: Apache-2.0

//! Artifact manifest verification for model-backed stages.
//!
//! A stage loads only from an operator-supplied directory that holds a
//! `lens-manifest.json`. Lens hashes the manifest's exact bytes and compares
//! the digest with the one the operator configured, then reads every listed
//! file once, checks its size and SHA-256 and keeps the bytes it will load.
//! Loading uses those verified bytes, so a file changed after verification
//! is never read. Verification downloads nothing. The optional fetcher
//! (`fetch` feature) checks a downloaded bundle with the same [`parse`] and
//! file checks before any stage loads it.
//!
//! The digest covers the manifest file's exact bytes. This is the provisional
//! canonical form until the model artifact owner freezes a serialization.
//! Diagnostics carry a fixed category and never a path, name or value.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Component, Path};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::parse_sha256;

/// Manifest file name inside an artifact directory.
pub(crate) const MANIFEST_FILE: &str = "lens-manifest.json";
/// The only manifest format this build reads.
pub(crate) const FORMAT: &str = "munarium-lens-manifest/0";
/// Largest manifest accepted, to bound the parse.
pub(crate) const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Most files one manifest may list.
const MAX_FILES: usize = 64;

/// Content-free reason a stage's artifacts were rejected.
// A fetch-only build parses manifests but loads no stage.
#[cfg_attr(not(feature = "qwen3"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactError {
    /// The manifest is missing, unreadable or too large.
    ManifestUnreadable,
    /// The manifest's SHA-256 is not the configured digest.
    ManifestDigestMismatch,
    /// The manifest does not parse or violates the schema.
    ManifestInvalid,
    /// The manifest names another stage, backend, precision or dimension.
    ManifestProfileMismatch,
    /// A listed path is absolute, escapes the directory or repeats.
    UnsafePath,
    /// A listed file is missing, not a regular file or unreadable.
    FileUnreadable,
    /// A listed file's size or SHA-256 differs from the manifest.
    FileDigestMismatch,
    /// A file the backend needs is not listed.
    RequiredFileMissing,
    /// The verified files did not load as the declared model.
    LoadFailed,
}

impl ArtifactError {
    pub(crate) fn category(self) -> &'static str {
        match self {
            Self::ManifestUnreadable => "manifest_unreadable",
            Self::ManifestDigestMismatch => "manifest_digest_mismatch",
            Self::ManifestInvalid => "manifest_invalid",
            Self::ManifestProfileMismatch => "manifest_profile_mismatch",
            Self::UnsafePath => "unsafe_path",
            Self::FileUnreadable => "file_unreadable",
            Self::FileDigestMismatch => "file_digest_mismatch",
            Self::RequiredFileMissing => "required_file_missing",
            Self::LoadFailed => "load_failed",
        }
    }
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.category())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Upstream {
    repository: String,
    revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEntry {
    path: String,
    size: u64,
    sha256: String,
}

// The profile fields are read only by stage loading (`qwen3`).
#[cfg_attr(not(feature = "qwen3"), allow(dead_code))]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    stage: String,
    backend: String,
    served_model: String,
    precision: String,
    #[serde(default)]
    dimensions: Option<usize>,
    upstream: Upstream,
    files: Vec<FileEntry>,
}

/// What the loading backend requires of a manifest.
#[cfg(feature = "qwen3")]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Expected<'a> {
    pub(crate) stage: &'a str,
    pub(crate) backend: &'a str,
    pub(crate) precision: &'a str,
    /// Required for embedding, absent for reranking.
    pub(crate) dimensions: Option<usize>,
    /// Files the backend loads. Each must be listed.
    pub(crate) required: &'a [&'a str],
}

/// A verified manifest with the bytes of the files the backend loads.
#[cfg(feature = "qwen3")]
#[derive(Debug)]
pub(crate) struct Verified {
    pub(crate) served_model: String,
    pub(crate) manifest_sha256: [u8; 32],
    files: BTreeMap<String, Vec<u8>>,
}

#[cfg(feature = "qwen3")]
impl Verified {
    /// Take the verified bytes of a required file.
    pub(crate) fn take(&mut self, path: &str) -> Result<Vec<u8>, ArtifactError> {
        self.files
            .remove(path)
            .ok_or(ArtifactError::RequiredFileMissing)
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A relative path of normal components only, with `/` separators.
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

/// Open a regular, non-symlink file of at most `limit` bytes.
fn open_regular(path: &Path, limit: u64) -> Option<(fs::File, u64)> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > limit {
        return None;
    }
    Some((fs::File::open(path).ok()?, metadata.len()))
}

/// Read at most `limit` bytes from a regular, non-symlink file.
pub(crate) fn read_regular(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let (file, length) = open_regular(path, limit)?;
    let mut bytes = Vec::with_capacity(usize::try_from(length).ok()?);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Stream at most `limit` bytes of a regular, non-symlink file through
/// SHA-256 without keeping them. Returns the length read and the digest.
pub(crate) fn hash_regular(path: &Path, limit: u64) -> Option<(u64, [u8; 32])> {
    let (file, _) = open_regular(path, limit)?;
    let mut reader = file.take(limit.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    (total <= limit).then(|| (total, hasher.finalize().into()))
}

/// One listed file: a safe relative path, its size and SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedFile {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) sha256: [u8; 32],
}

/// A manifest whose digest, schema and paths passed. The stage profile and
/// the files themselves are not yet checked.
#[derive(Debug)]
pub(crate) struct Listing {
    manifest: Manifest,
    /// Listed files in path order.
    pub(crate) files: Vec<ListedFile>,
}

impl Listing {
    #[cfg(feature = "fetch")]
    pub(crate) fn served_model(&self) -> &str {
        &self.manifest.served_model
    }
}

/// Check manifest bytes against `expected_sha256`, then the schema and every
/// listed path. Loading and the fetcher both start here, so a downloaded
/// bundle meets exactly the checks of an operator-supplied one.
pub(crate) fn parse(
    manifest_bytes: &[u8],
    expected_sha256: &[u8; 32],
) -> Result<Listing, ArtifactError> {
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ArtifactError::ManifestUnreadable);
    }
    if &sha256(manifest_bytes) != expected_sha256 {
        return Err(ArtifactError::ManifestDigestMismatch);
    }
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| ArtifactError::ManifestInvalid)?;
    let model_ok = !manifest.served_model.is_empty()
        && manifest.served_model.len() <= 128
        && manifest
            .served_model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b));
    if manifest.format != FORMAT
        || !model_ok
        || manifest.upstream.repository.is_empty()
        || manifest.upstream.revision.is_empty()
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
    {
        return Err(ArtifactError::ManifestInvalid);
    }
    let mut listed = BTreeMap::new();
    for entry in &manifest.files {
        if !safe_relative(&entry.path) || listed.contains_key(entry.path.as_str()) {
            return Err(ArtifactError::UnsafePath);
        }
        let digest = parse_sha256(&entry.sha256).ok_or(ArtifactError::ManifestInvalid)?;
        listed.insert(entry.path.as_str(), (entry.size, digest));
    }
    let files = listed
        .into_iter()
        .map(|(path, (size, sha256))| ListedFile {
            path: path.to_owned(),
            size,
            sha256,
        })
        .collect();
    Ok(Listing { manifest, files })
}

/// Check every listed file under `directory`: a regular file, not a symlink,
/// of exactly the listed size and SHA-256. Keeps the bytes of the files
/// `keep` selects and streams the others through the hash.
fn check_files(
    directory: &Path,
    listing: &Listing,
    keep: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<u8>>, ArtifactError> {
    let mut files = BTreeMap::new();
    for file in &listing.files {
        let path = directory.join(&file.path);
        if keep(&file.path) {
            let bytes = read_regular(&path, file.size).ok_or(ArtifactError::FileUnreadable)?;
            if bytes.len() as u64 != file.size || sha256(&bytes) != file.sha256 {
                return Err(ArtifactError::FileDigestMismatch);
            }
            files.insert(file.path.clone(), bytes);
        } else {
            let (length, digest) =
                hash_regular(&path, file.size).ok_or(ArtifactError::FileUnreadable)?;
            if length != file.size || digest != file.sha256 {
                return Err(ArtifactError::FileDigestMismatch);
            }
        }
    }
    Ok(files)
}

/// Read `lens-manifest.json` from `directory` and verify it and every file
/// it lists against `expected_sha256`.
#[cfg(feature = "qwen3")]
pub(crate) fn verify_directory(
    directory: &Path,
    expected_sha256: &[u8; 32],
    expected: Expected<'_>,
) -> Result<Verified, ArtifactError> {
    let manifest = read_regular(&directory.join(MANIFEST_FILE), MAX_MANIFEST_BYTES)
        .ok_or(ArtifactError::ManifestUnreadable)?;
    verify(directory, &manifest, expected_sha256, expected)
}

/// Verify manifest bytes and the files they list under `directory`.
#[cfg(feature = "qwen3")]
pub(crate) fn verify(
    directory: &Path,
    manifest_bytes: &[u8],
    expected_sha256: &[u8; 32],
    expected: Expected<'_>,
) -> Result<Verified, ArtifactError> {
    let listing = parse(manifest_bytes, expected_sha256)?;
    let manifest = &listing.manifest;
    if manifest.stage != expected.stage
        || manifest.backend != expected.backend
        || manifest.precision != expected.precision
        || manifest.dimensions != expected.dimensions
    {
        return Err(ArtifactError::ManifestProfileMismatch);
    }
    if !expected
        .required
        .iter()
        .all(|path| listing.files.iter().any(|file| file.path == *path))
    {
        return Err(ArtifactError::RequiredFileMissing);
    }
    let files = check_files(directory, &listing, |path| {
        expected.required.contains(&path)
    })?;
    Ok(Verified {
        served_model: listing.manifest.served_model,
        manifest_sha256: *expected_sha256,
        files,
    })
}

/// Verify `lens-manifest.json` in `directory` and every file it lists, with
/// the shared [`parse`] and file checks but no stage profile, keeping no
/// file bytes. Returns the manifest's served name. This is the fetcher's
/// final check; loading a stage still applies the full stage verification.
#[cfg(feature = "fetch")]
pub(crate) fn verify_files(
    directory: &Path,
    expected_sha256: &[u8; 32],
) -> Result<String, ArtifactError> {
    let manifest = read_regular(&directory.join(MANIFEST_FILE), MAX_MANIFEST_BYTES)
        .ok_or(ArtifactError::ManifestUnreadable)?;
    let listing = parse(&manifest, expected_sha256)?;
    check_files(directory, &listing, |_| false)?;
    Ok(listing.manifest.served_model)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch directory removed on drop. Only synthetic files go in it.
    pub(crate) struct Scratch(pub(crate) PathBuf);

    impl Scratch {
        pub(crate) fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "munarium-lens-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn hex(digest: &[u8; 32]) -> String {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn parse_requires_the_pinned_manifest_digest() {
        // Shared by stage loading and the fetcher, so checked in every build
        // that has this module.
        let bytes = format!(
            r#"{{"format":"{FORMAT}","stage":"embedding","backend":"qwen3","served_model":"m","precision":"f32","upstream":{{"repository":"r","revision":"v"}},"files":[{{"path":"a","size":0,"sha256":"{}"}}]}}"#,
            hex(&sha256(b""))
        )
        .into_bytes();
        let listing = parse(&bytes, &sha256(&bytes)).unwrap();
        assert_eq!(listing.files.len(), 1);
        assert_eq!(
            parse(&bytes, &[0; 32]).unwrap_err(),
            ArtifactError::ManifestDigestMismatch
        );
        let mut changed = bytes.clone();
        changed.push(b'\n');
        assert_eq!(
            parse(&changed, &sha256(&bytes)).unwrap_err(),
            ArtifactError::ManifestDigestMismatch
        );
    }

    #[test]
    fn parses_only_lowercase_sha256_hex() {
        let digest = sha256(b"abc");
        assert_eq!(parse_sha256(&hex(&digest)), Some(digest));
        assert_eq!(parse_sha256(&hex(&digest).to_uppercase()), None);
        assert_eq!(parse_sha256("ab"), None);
        assert_eq!(parse_sha256(&"g".repeat(64)), None);
    }

    #[cfg(feature = "qwen3")]
    mod stage {
        use super::*;

        const EXPECTED: Expected<'static> = Expected {
            stage: "embedding",
            backend: "qwen3",
            precision: "f32",
            dimensions: Some(4),
            required: &["config.json", "weights.bin"],
        };

        fn entry(path: &str, bytes: &[u8]) -> String {
            format!(
                r#"{{"path":{},"size":{},"sha256":"{}"}}"#,
                serde_json::to_string(path).unwrap(),
                bytes.len(),
                hex(&sha256(bytes))
            )
        }

        fn manifest(stage: &str, entries: &[String]) -> Vec<u8> {
            format!(
            r#"{{"format":"{FORMAT}","stage":"{stage}","backend":"qwen3","served_model":"fixture/embedding-4d","precision":"f32","dimensions":4,"upstream":{{"repository":"fixture/model","revision":"0000"}},"files":[{}]}}"#,
            entries.join(",")
        )
        .into_bytes()
        }

        fn fixture() -> (Scratch, Vec<String>) {
            let scratch = Scratch::new();
            fs::write(scratch.0.join("config.json"), b"{}").unwrap();
            fs::create_dir_all(scratch.0.join("sub")).unwrap();
            fs::write(scratch.0.join("weights.bin"), b"synthetic weights").unwrap();
            fs::write(
                scratch.0.join("sub/notes.txt"),
                b"listed, verified, not loaded",
            )
            .unwrap();
            let entries = vec![
                entry("config.json", b"{}"),
                entry("weights.bin", b"synthetic weights"),
                entry("sub/notes.txt", b"listed, verified, not loaded"),
            ];
            (scratch, entries)
        }

        fn check(dir: &Path, bytes: &[u8]) -> Result<Verified, ArtifactError> {
            verify(dir, bytes, &sha256(bytes), EXPECTED)
        }

        #[test]
        fn verifies_listed_files_and_keeps_only_required_bytes() {
            let (scratch, entries) = fixture();
            let bytes = manifest("embedding", &entries);
            fs::write(scratch.0.join(MANIFEST_FILE), &bytes).unwrap();
            let mut verified = verify_directory(&scratch.0, &sha256(&bytes), EXPECTED).unwrap();
            assert_eq!(verified.served_model, "fixture/embedding-4d");
            assert_eq!(verified.manifest_sha256, sha256(&bytes));
            assert_eq!(verified.take("weights.bin").unwrap(), b"synthetic weights");
            assert_eq!(
                verified.take("sub/notes.txt"),
                Err(ArtifactError::RequiredFileMissing)
            );
        }

        #[test]
        fn rejects_a_manifest_that_is_not_the_configured_one() {
            let (scratch, entries) = fixture();
            let bytes = manifest("embedding", &entries);
            assert_eq!(
                verify(&scratch.0, &bytes, &[0; 32], EXPECTED).unwrap_err(),
                ArtifactError::ManifestDigestMismatch
            );
            assert_eq!(
                verify_directory(&scratch.0, &sha256(&bytes), EXPECTED).unwrap_err(),
                ArtifactError::ManifestUnreadable
            );
        }

        #[test]
        fn rejects_changed_missing_or_resized_files() {
            let (scratch, entries) = fixture();
            let bytes = manifest("embedding", &entries);
            fs::write(scratch.0.join("weights.bin"), b"synthetic weightz").unwrap();
            assert_eq!(
                check(&scratch.0, &bytes).unwrap_err(),
                ArtifactError::FileDigestMismatch
            );
            fs::write(scratch.0.join("weights.bin"), b"synthetic weights, longer").unwrap();
            assert_eq!(
                check(&scratch.0, &bytes).unwrap_err(),
                ArtifactError::FileUnreadable
            );
            fs::write(scratch.0.join("weights.bin"), b"short").unwrap();
            assert_eq!(
                check(&scratch.0, &bytes).unwrap_err(),
                ArtifactError::FileDigestMismatch
            );
            fs::remove_file(scratch.0.join("weights.bin")).unwrap();
            assert_eq!(
                check(&scratch.0, &bytes).unwrap_err(),
                ArtifactError::FileUnreadable
            );
        }

        #[test]
        fn rejects_unsafe_or_repeated_paths_and_unlisted_required_files() {
            let (scratch, entries) = fixture();
            for path in [
                "../config.json",
                "/etc/passwd",
                "C:/x",
                "sub\\notes.txt",
                "./config.json",
                "",
            ] {
                let mut listed = entries.clone();
                listed.push(entry(path, b"{}"));
                assert_eq!(
                    check(&scratch.0, &manifest("embedding", &listed)).unwrap_err(),
                    ArtifactError::UnsafePath,
                    "{path}"
                );
            }
            let mut repeated = entries.clone();
            repeated.push(entries[0].clone());
            assert_eq!(
                check(&scratch.0, &manifest("embedding", &repeated)).unwrap_err(),
                ArtifactError::UnsafePath
            );
            assert_eq!(
                check(&scratch.0, &manifest("embedding", &entries[..1])).unwrap_err(),
                ArtifactError::RequiredFileMissing
            );
        }

        #[test]
        fn rejects_another_profile_unknown_fields_and_bad_digests() {
            let (scratch, entries) = fixture();
            assert_eq!(
                check(&scratch.0, &manifest("reranking", &entries)).unwrap_err(),
                ArtifactError::ManifestProfileMismatch
            );
            let mut extra = String::from_utf8(manifest("embedding", &entries)).unwrap();
            extra.insert_str(1, r#""loader":"remote","#);
            assert_eq!(
                check(&scratch.0, extra.as_bytes()).unwrap_err(),
                ArtifactError::ManifestInvalid
            );
            let upper = String::from_utf8(manifest("embedding", &entries))
                .unwrap()
                .to_uppercase();
            assert!(check(&scratch.0, upper.as_bytes()).is_err());
            assert_eq!(
                check(&scratch.0, b"not json").unwrap_err(),
                ArtifactError::ManifestInvalid
            );
        }

        #[cfg(unix)]
        #[test]
        fn rejects_a_symlinked_file() {
            let (scratch, entries) = fixture();
            fs::rename(scratch.0.join("weights.bin"), scratch.0.join("real.bin")).unwrap();
            std::os::unix::fs::symlink(scratch.0.join("real.bin"), scratch.0.join("weights.bin"))
                .unwrap();
            assert_eq!(
                check(&scratch.0, &manifest("embedding", &entries)).unwrap_err(),
                ArtifactError::FileUnreadable
            );
        }
    }
}
