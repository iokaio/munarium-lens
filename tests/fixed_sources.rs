// SPDX-License-Identifier: Apache-2.0

//! A built Lens can download only from the two fixed Munarium locations, in
//! a fixed order. These tests compile the library as a release or image
//! build does, without `cfg(test)`, so the test-only endpoint injection
//! inside the crate does not exist here.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use munarium_lens::Config;

/// Every variable configuration reads. A new variable, for example a base
/// URL, mirror or source order setting, fails this test until reviewed.
const READ: &[&str] = &[
    "LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK",
    "LENS_EMBEDDING_ARTIFACTS",
    "LENS_EMBEDDING_BACKEND",
    "LENS_EMBEDDING_BUNDLE",
    "LENS_EMBEDDING_BUNDLE_REVISION",
    "LENS_EMBEDDING_MANIFEST_SHA256",
    "LENS_HOST",
    "LENS_MAX_BATCH_TOKENS",
    "LENS_MAX_BODY_BYTES",
    "LENS_MAX_INPUT_TOKENS",
    "LENS_MAX_IN_FLIGHT",
    "LENS_MAX_ITEMS",
    "LENS_MAX_QUEUE",
    "LENS_MODEL_CACHE_DIR",
    "LENS_MODEL_FETCH_TIMEOUT_SECS",
    "LENS_PORT",
    "LENS_QUEUE_TIMEOUT_MS",
    "LENS_REFERENCE_DIMENSIONS",
    "LENS_RERANKING_ARTIFACTS",
    "LENS_RERANKING_BACKEND",
    "LENS_RERANKING_BUNDLE",
    "LENS_RERANKING_BUNDLE_REVISION",
    "LENS_RERANKING_MANIFEST_SHA256",
];

fn variables_read(values: &[(&str, &str)]) -> BTreeSet<String> {
    let read = Mutex::new(BTreeSet::new());
    let _ = Config::from_lookup(|name| {
        read.lock().unwrap().insert(name.to_owned());
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    });
    read.into_inner().unwrap()
}

#[test]
fn configuration_reads_no_endpoint_or_source_setting() {
    let expected: BTreeSet<String> = READ.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(variables_read(&[]), expected);
    #[cfg(all(feature = "fetch", feature = "qwen3"))]
    {
        let digest = "ab".repeat(32);
        let revision = "0123456789abcdef0123456789abcdef01234567";
        let full = [
            ("LENS_EMBEDDING_BACKEND", "qwen3"),
            ("LENS_EMBEDDING_BUNDLE", "qwen3-embedding-0.6b-f32-1024d"),
            ("LENS_EMBEDDING_BUNDLE_REVISION", revision),
            ("LENS_EMBEDDING_MANIFEST_SHA256", digest.as_str()),
            ("LENS_RERANKING_BACKEND", "qwen3"),
            ("LENS_RERANKING_BUNDLE", "qwen3-reranker-0.6b-f32"),
            ("LENS_RERANKING_BUNDLE_REVISION", revision),
            ("LENS_RERANKING_MANIFEST_SHA256", digest.as_str()),
            ("LENS_MODEL_CACHE_DIR", "/var/cache/lens"),
            ("LENS_MODEL_FETCH_TIMEOUT_SECS", "600"),
        ];
        let config = Config::from_lookup(|name| {
            full.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
        .unwrap();
        assert!(config.embedding_bundle.is_some() && config.model_cache.is_some());
        assert_eq!(variables_read(&full), expected);
    }
}

/// Outside configuration, non-test source never reads the environment, so
/// no module can take a base URL, mirror or order from it. Each file's
/// trailing `#[cfg(test)] mod tests` block is excluded.
#[test]
fn only_configuration_reads_the_environment() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in fs::read_dir(&source).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|name| name == "config.rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
        let text = [
            "\n#[cfg(test)]\nmod tests {",
            "\n#[cfg(test)]\npub(crate) mod tests {",
        ]
        .iter()
        .filter_map(|marker| text.find(marker))
        .min()
        .map_or(text.as_str(), |end| &text[..end]);
        for pattern in ["env::var", "env::vars", "var_os(", "option_env!", "env!("] {
            assert!(
                !text.contains(pattern),
                "{} reads the environment ({pattern})",
                path.display()
            );
        }
    }
}

#[cfg(feature = "fetch")]
mod fetch {
    use std::fs;
    use std::path::Path;

    use munarium_lens::fetch::{
        GITHUB_HOSTS, GITHUB_RELEASES_BASE, HUGGING_FACE_BASE, HUGGING_FACE_HOSTS, SOURCE_ORDER,
        Source, file_url, parse_command,
    };

    const REVISION: &str = "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3";

    #[test]
    fn the_two_locations_are_fixed() {
        assert_eq!(
            HUGGING_FACE_BASE,
            "https://huggingface.co/iokaio/munarium-lens-models"
        );
        assert_eq!(
            GITHUB_RELEASES_BASE,
            "https://github.com/iokaio/munarium-lens/releases/download/"
        );
        assert_eq!(SOURCE_ORDER, [Source::HuggingFace, Source::GitHubReleases]);
        assert_eq!(
            file_url(Source::HuggingFace, REVISION, "b", "f.json").unwrap(),
            format!("{HUGGING_FACE_BASE}/resolve/{REVISION}/b/f.json")
        );
        assert_eq!(
            file_url(Source::GitHubReleases, REVISION, "b", "f.json").unwrap(),
            format!("{GITHUB_RELEASES_BASE}models/b/f.json")
        );
    }

    #[test]
    fn redirects_stay_on_the_documented_hosts() {
        assert_eq!(
            HUGGING_FACE_HOSTS,
            [
                "huggingface.co",
                "cas-bridge.xethub.hf.co",
                "cdn-lfs.hf.co",
                "cdn-lfs-us-1.hf.co",
                "cdn-lfs-eu-1.hf.co",
            ]
        );
        assert_eq!(
            GITHUB_HOSTS,
            [
                "github.com",
                "release-assets.githubusercontent.com",
                "objects.githubusercontent.com",
            ]
        );
    }

    #[test]
    fn the_command_takes_no_endpoint_or_source_option() {
        for flag in [
            "--source",
            "--sources",
            "--base-url",
            "--mirror",
            "--url",
            "--endpoint",
        ] {
            let args = vec![flag.to_owned(), "x".to_owned()];
            assert_eq!(parse_command(&args), Err("unknown argument"), "{flag}");
        }
    }

    /// The test-only endpoint injection exists only in `cfg(test)` code.
    #[test]
    fn endpoint_injection_is_test_only() {
        let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fetch.rs"))
            .unwrap()
            .replace("\r\n", "\n");
        let tests_start = text
            .find("#[cfg(test)]\npub(crate) mod tests {")
            .expect("the test module");
        for name in [
            "fn override_base",
            "fn test_endpoint",
            "struct UseEndpoints",
        ] {
            let at = text.find(name).expect(name);
            assert!(at > tests_start, "{name} must be inside the test module");
        }
        for call in ["tests::override_base(", "tests::test_endpoint("] {
            for (at, _) in text.match_indices(call) {
                let before = &text[..at];
                let guard = before.rfind("#[cfg(test)]").expect("a cfg(test) guard");
                let between = &before[guard..];
                assert!(
                    between.lines().count() <= 3,
                    "{call} must be directly under #[cfg(test)]"
                );
            }
        }
    }
}
