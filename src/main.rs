// SPDX-License-Identifier: Apache-2.0

use tokio::net::TcpListener;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
