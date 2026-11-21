// SPDX-License-Identifier: Apache-2.0

use tokio::net::TcpListener;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        Some(first) if first == "fetch" => std::process::exit(fetch(args.collect())),
        Some(first) if first == "import" => std::process::exit(import(args.collect())),
        _ => serve(),
    }
}

#[cfg(feature = "fetch")]
fn import(args: Vec<std::ffi::OsString>) -> i32 {
    use std::path::Path;
    let [source, cache, digest] = args.as_slice() else {
        eprintln!(
            "usage: munarium-lens import <source-directory> <cache-directory> <manifest-sha256>"
        );
        return 2;
    };
    let Some(digest) = digest.to_str() else {
        eprintln!("manifest-sha256 must be a lowercase hex SHA-256");
        return 2;
    };
    match munarium_lens::fetch::import_bundle(Path::new(source), Path::new(cache), digest) {
        Ok(()) => {
            println!("{digest}");
            0
        }
        Err(error) => {
            eprintln!("Lens import failed: {error}");
            1
        }
    }
}

#[cfg(not(feature = "fetch"))]
fn import(_: Vec<std::ffi::OsString>) -> i32 {
    eprintln!("this build has no bundle importer; rebuild with the `fetch` cargo feature");
    2
}

/// `munarium-lens fetch`: download and verify a pinned bundle, then exit.
#[cfg(feature = "fetch")]
fn fetch(args: Vec<std::ffi::OsString>) -> i32 {
    let Some(args) = args
        .into_iter()
        .map(|arg| arg.into_string().ok())
        .collect::<Option<Vec<String>>>()
    else {
        eprintln!("arguments must be UTF-8\n{}", munarium_lens::fetch::USAGE);
        return 2;
    };
    // The standard HTTPS proxy settings, read by configuration like every
    // other variable. A transport route only: the sources stay fixed.
    let proxy = match munarium_lens::FetchProxy::from_env() {
        Ok(proxy) => proxy,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    munarium_lens::fetch::run_command(&args, proxy.as_ref())
}

#[cfg(not(feature = "fetch"))]
fn fetch(_: Vec<std::ffi::OsString>) -> i32 {
    eprintln!("this build has no fetcher; rebuild with the `fetch` cargo feature");
    2
}

#[tokio::main(flavor = "current_thread")]
async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let config = munarium_lens::Config::from_env()?;
    #[cfg(unix)]
    let (mut interrupt, mut terminate) = {
        use tokio::signal::unix::{SignalKind, signal};
        (
            signal(SignalKind::interrupt())?,
            signal(SignalKind::terminate())?,
        )
    };
    let listener = TcpListener::bind(config.listen).await?;

    munarium_lens::serve_with(listener, &config, async move {
        #[cfg(unix)]
        tokio::select! {
            _ = interrupt.recv() => {},
            _ = terminate.recv() => {},
        }
        #[cfg(not(unix))]
        if tokio::signal::ctrl_c().await.is_err() {
            eprintln!("shutdown signal unavailable");
        }
    })
    .await?;
    Ok(())
}
