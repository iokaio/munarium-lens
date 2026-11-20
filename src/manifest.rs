// SPDX-License-Identifier: Apache-2.0

//! Artifact manifest verification for model-backed stages.
//!
//! A stage loads only from an operator-supplied directory that holds a
//! `lens-manifest.json`. Lens hashes the manifest's exact bytes and compares
//! the digest with the one the operator configured, then reads every listed
//! file once, checks its size and SHA-256 and keeps the bytes it will load.
//! Loading uses those verified bytes, so a file changed after verification
//! is never read. Nothing is downloaded.
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
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Most files one manifest may list.
const MAX_FILES: usize = 64;

/// Content-free reason a stage's artifacts were rejected.
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
#[derive(Debug)]
pub(crate) struct Verified {
    pub(crate) served_model: String,
    pub(crate) manifest_sha256: [u8; 32],
    files: BTreeMap<String, Vec<u8>>,
}

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

/// Read at most `limit` bytes from a regular, non-symlink file.
fn read_regular(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).ok()?);
    fs::File::open(path)
        .ok()?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Read `lens-manifest.json` from `directory` and verify it and every file
/// it lists against `expected_sha256`.
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
pub(crate) fn verify(
    directory: &Path,
    manifest_bytes: &[u8],
    expected_sha256: &[u8; 32],
    expected: Expected<'_>,
) -> Result<Verified, ArtifactError> {
    let manifest_sha256 = sha256(manifest_bytes);
    if &manifest_sha256 != expected_sha256 {
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
    if manifest.stage != expected.stage
        || manifest.backend != expected.backend
        || manifest.precision != expected.precision
        || manifest.dimensions != expected.dimensions
    {
        return Err(ArtifactError::ManifestProfileMismatch);
    }
    let mut listed = BTreeMap::new();
    for entry in &manifest.files {
        if !safe_relative(&entry.path) || listed.contains_key(entry.path.as_str()) {
            return Err(ArtifactError::UnsafePath);
        }
        let digest = parse_sha256(&entry.sha256).ok_or(ArtifactError::ManifestInvalid)?;
        listed.insert(entry.path.as_str(), (entry.size, digest));
    }
    if !expected
        .required
        .iter()
        .all(|path| listed.contains_key(path))
    {
        return Err(ArtifactError::RequiredFileMissing);
    }
    let mut files = BTreeMap::new();
    for (path, (size, digest)) in listed {
        let bytes =
            read_regular(&directory.join(path), size).ok_or(ArtifactError::FileUnreadable)?;
        if bytes.len() as u64 != size || sha256(&bytes) != digest {
            return Err(ArtifactError::FileDigestMismatch);
        }
        if expected.required.contains(&path) {
            files.insert(path.to_owned(), bytes);
        }
    }
    Ok(Verified {
        served_model: manifest.served_model,
        manifest_sha256,
        files,
    })
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

    #[test]
    fn parses_only_lowercase_sha256_hex() {
        let digest = sha256(b"abc");
        assert_eq!(parse_sha256(&hex(&digest)), Some(digest));
        assert_eq!(parse_sha256(&hex(&digest).to_uppercase()), None);
        assert_eq!(parse_sha256("ab"), None);
        assert_eq!(parse_sha256(&"g".repeat(64)), None);
    }
}
