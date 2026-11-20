// SPDX-License-Identifier: Apache-2.0

use tokio::net::TcpListener;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|first| first == "fetch") {
        std::process::exit(fetch(args.collect()));
    }
    serve()
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
    munarium_lens::fetch::run_command(&args)
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
