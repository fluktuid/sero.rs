mod scaler;
mod settings;

use anyhow::Result;
use std::sync::Arc;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Notify,
    time::Duration,
};
use tracing::{error, info, trace};
use tokio::signal;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let conf = settings::Settings::new();

    let settings = conf.unwrap();

    let listen_address = settings.host;
    let name = settings.target.service.name;
    let backend_address = format!("{}:{}", name, settings.target.service.port.to_string());

    let target_deploy = settings.target.deployment;

    let listener = TcpListener::bind(&listen_address).await?;

    info!("Listening on {listen_address}.");
    info!("Proxying requests to {backend_address}.");
    info!("Target deployment is {target_deploy}.");

    let backend_unavailable = Arc::new(Notify::new());
    let backend_available = Arc::new(Notify::new());

    let (shutdown_send, mut shutdown_recv) = mpsc::unbounded_channel();
    {
        let backend_unavailable = backend_unavailable.clone();
        let backend_available = backend_available.clone();
        let handle = tokio::spawn(async move {
            loop {
                backend_unavailable.notified().await;
                info!("Got a request to a backend that is unreachable. Trying to scale up.");
                while let Err(e) =
                    scaler::scale_deploy(&target_deploy, 1, Duration::from_secs(10)).await
                {
                    error!("Failed to scale up: {e}");
                }
                backend_available.notify_waiters();
                info!("Backend is up again.");
            }
        });
        tokio::spawn(async move {
            match signal::ctrl_c().await {
                Ok(()) => {
                    info!("received kill signal");
                    handle.abort();
                    _ = shutdown_send.send(0);
                },
                Err(err) => {
                    eprintln!("Unable to listen for shutdown signal: {}", err);
                    // we also shut down in case of error
                },
            }
        });
    }

    tokio::select! {
        _ = run_listener(&listener, &backend_unavailable, &backend_available, &backend_address) => {},
        _ = shutdown_recv.recv() => {},
    }

    Ok(())
}

async fn proxy_connection(mut ingress: TcpStream, mut backend: TcpStream) {
    match tokio::io::copy_bidirectional(&mut ingress, &mut backend).await {
        Ok((bytes_to_backend, bytes_from_backend)) => {
            trace!("Connection ended gracefully ({bytes_to_backend} bytes from client, {bytes_from_backend} bytes from server)");
        }
        Err(e) => {
            error!("Error while proxying: {e}");
        }
    }
}

async fn run_listener(listener: &TcpListener, backend_unavailable: &Arc<Notify>, backend_available: &Arc<Notify>, backend_address: &str) {
    {
        while let Ok((ingress, _)) = listener.accept().await {
            let backend_unavailable = backend_unavailable.clone();
            let backend_available = backend_available.clone();
            let backend_address = backend_address.to_owned().clone();
            // span a new task to handle the connection
            tokio::spawn(async move {
                loop {
                    match TcpStream::connect(&backend_address).await {
                        Ok(backend) => {
                            trace!("Successfully connected to backend. Proxying packets.");
                            proxy_connection(ingress, backend).await;
                            break;
                        }
                        Err(_) => {
                            backend_unavailable.notify_one();
                            backend_available.notified().await;
                        }
                    };
                }
            });
        }
    }
}