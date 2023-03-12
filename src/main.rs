mod scaler;

use anyhow::Result;
use std::sync::Arc;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Notify,
    time::Duration,
};

#[tokio::main]
async fn main() -> Result<()> {
    let listen_address = "0.0.0.0:3000";
    let backend_address = "127.0.0.1:8000";

    let listener = TcpListener::bind(&listen_address).await?;
    println!("Listening on {listen_address}.");

    let backend_unavailable = Arc::new(Notify::new());
    let backend_available = Arc::new(Notify::new());

    {
        let backend_unavailable = backend_unavailable.clone();
        let backend_available = backend_available.clone();
        tokio::spawn(async move {
            loop {
                backend_unavailable.notified().await;
                println!("Got a request to a backend that is unreachable. Trying to scale up.");
                while let Err(e) =
                    scaler::scale_deploy("wasm-spin", 1, Duration::from_secs(10)).await
                {
                    println!("Failed to scale up: {e}");
                }
                backend_available.notify_waiters();
                println!("Backend is up again.");
            }
        });
    }

    // handle incoming connections
    while let Ok((ingress, _)) = listener.accept().await {
        let backend_unavailable = backend_unavailable.clone();
        let backend_available = backend_available.clone();
        // span a new task to handle the connection
        tokio::spawn(async move {
            loop {
                match TcpStream::connect(&backend_address).await {
                    Ok(backend) => {
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

    Ok(())
}

async fn proxy_connection(mut ingress: TcpStream, mut backend: TcpStream) {
    match tokio::io::copy_bidirectional(&mut ingress, &mut backend).await {
        Ok((bytes_to_backend, bytes_from_backend)) => {
            println!("Connection ended gracefully ({bytes_to_backend} bytes from client, {bytes_from_backend} bytes from server)");
        }
        Err(e) => {
            println!("Error while proxying: {e}");
        }
    }
}
