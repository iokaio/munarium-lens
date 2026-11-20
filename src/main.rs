// SPDX-License-Identifier: Apache-2.0

use std::net::Ipv4Addr;

use tokio::net::TcpListener;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = match std::env::var("LENS_PORT") {
        Ok(value) => value.parse::<u16>()?,
        Err(std::env::VarError::NotPresent) => 8787,
        Err(error) => return Err(error.into()),
    };
    #[cfg(unix)]
    let (mut interrupt, mut terminate) = {
        use tokio::signal::unix::{SignalKind, signal};
        (
            signal(SignalKind::interrupt())?,
            signal(SignalKind::terminate())?,
        )
    };
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("Lens scaffold listening on {}", listener.local_addr()?);

    munarium_lens::serve(listener, async move {
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
