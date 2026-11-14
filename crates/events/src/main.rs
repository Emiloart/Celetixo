use celetixo_events::{EventError, worker};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[tokio::main]
async fn main() {
    let result = execute().await;
    if let Err(error) = result {
        eprintln!("celetixo-events: {error}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), EventError> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--config")) {
        return Err(EventError::Configuration);
    }
    let path = PathBuf::from(args.next().ok_or(EventError::Configuration)?);
    if args.next().is_some() {
        return Err(EventError::Configuration);
    }
    let config = tokio::task::spawn_blocking(move || worker::load(&path))
        .await
        .map_err(|_| EventError::DatabaseTask)??;
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let handler = tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .map_err(|_| EventError::Configuration)?;
            tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
        }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c()
            .await
            .map_err(|_| EventError::Configuration)?;
        signal.store(true, Ordering::Release);
        Ok::<_, EventError>(())
    });
    let result = worker::run(config, cancel).await;
    handler.abort();
    result
}
