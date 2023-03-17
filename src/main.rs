mod scaler;
mod settings;
mod slicer;
mod toggle;

use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::{
    net::{TcpListener, TcpStream},
    time::Duration,
};
use kube::Client;
use tracing::{error, info, trace, warn};
use tokio::signal;
use tokio::sync::mpsc;
use std::time::{SystemTime};
use std::thread::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();
    let last_seen = Arc::new(Mutex::new(now));

    let conf = settings::Settings::new();

    let settings = conf.unwrap();

    let listen_address = settings.host;
    let name = settings.target.service.name;
    let port = settings.target.service.port.to_string();
    let scale_down = settings.target.timeout.scale_down;
    let backend_address = format!("{}:{}", name, port);

    let target_deploy = settings.target.deployment;

    let listener = TcpListener::bind(&listen_address).await?;

    let client = Client::try_default().await?;

    info!("Listening on {listen_address}.");
    info!("Proxying requests to {backend_address}.");
    info!("Target deployment is {target_deploy}.");

    let slicer = Arc::new(slicer::Slice::new(&target_deploy, &name, &client));
    let slice_name = slicer.apply_slice().await.unwrap();
    info!("created slice {slice_name}");

    let backend_toggle = Arc::new(toggle::Toggle::new(true));

    let (shutdown_send, mut shutdown_recv) = mpsc::unbounded_channel();
    {
        let backend_toggle = backend_toggle.clone();
        let td = target_deploy.clone();
        let a_slicer = slicer.clone();
        let handle = tokio::spawn(async move {
            loop {
                // backend_unavailable.notified().await;
                backend_toggle.wait_for(false).await;
                info!("Got a request to a backend that is unreachable. Trying to scale up.");
                while let Err(e) =
                    scaler::scale_deploy(&target_deploy, 1, Duration::from_secs(10)).await
                {
                    error!("Failed to scale up: {e}");
                }
                _ = a_slicer.remove_ep_from_svc().await;
                info!("scaling revoving ep from slice");
                backend_toggle.set(true).await;
                info!("Backend is up again.");
            }
        });
        let scale_down = scale_down.clone();
        let last_seen = last_seen.clone();
        let slicer = slicer.clone();
        let handle_scale_down = tokio::spawn(async move {
            let dur = Duration::from_millis(scale_down);
            loop {
                // backend_available.notified().await;
                let sleep_duration: u64;
                match last_seen.lock() {
                    Ok(seen) => {
                        let millis: u64 = dur.as_millis() as u64;
                        let dur = *seen as u64 + millis - now_unix_millis() as u64;
                        if dur > millis {
                            sleep_duration = millis;
                        } else {
                            sleep_duration = dur;
                        }
                        drop(seen);
                    },
                    Err(_) => {sleep_duration = dur.as_millis() as u64},
                };

                let secs = sleep_duration/1000;
                info!("sleeping for {}s", secs);
                sleep(Duration::from_millis(sleep_duration));
                info!("scale down loop woke up");
                let scale_down: bool;
                match last_seen.lock() {
                    Ok(seen) => {
                        scale_down = *seen + dur.as_millis() <= now_unix_millis();
                        drop(seen);
                    },
                    Err(_) => {
                        warn!("failed getting last_seen; can't calculate scale_down.");
                        continue;
                    },
                }
                if scale_down {
                    info!("scaling down");
                    _ = slicer.append_ep_to_svc().await;
                    while let Err(e) =
                        scaler::scale_deploy(&td, 0, Duration::from_secs(10)).await
                    {
                        error!("Failed to scale up: {e}");
                    }
                }
            }
        });
        tokio::spawn(async move {
            match signal::ctrl_c().await {
                Ok(()) => {
                    info!("received kill signal");
                    handle.abort();
                    handle_scale_down.abort();
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
        _ = run_listener(&listener, &backend_toggle, &backend_address, &last_seen) => {},
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

async fn run_listener(listener: &TcpListener, backend_toggle: &Arc<toggle::Toggle>, backend_address: &str, last_seen: &Arc<Mutex<u128>>) {
    {
        while let Ok((ingress, _)) = listener.accept().await {
            let backend_toggle = backend_toggle.clone();
            let backend_address = backend_address.to_owned().clone();
            let last_seen = last_seen.clone();
            // span a new task to handle the connection
            tokio::spawn(async move {
                loop {
                    if backend_toggle.get().await == false {
                        backend_toggle.set(false).await;
                        backend_toggle.wait_for(true).await;
                    }
                    match TcpStream::connect(&backend_address).await {
                        Ok(backend) => {
                            trace!("Successfully connected to backend. Proxying packets.");
                            match last_seen.lock() {
                                Ok(mut lock) => {
                                    *lock = now_unix_millis();
                                    drop(lock);
                                },
                                Err(_) => {},
                            };
                            proxy_connection(ingress, backend).await;
                            break;
                        }
                        Err(_) => {
                            _ = backend_toggle.set(false).await;
                            backend_toggle.wait_for(true).await;
                        }
                    };
                }
            });
        }
    }
}

fn now_unix_millis() -> u128 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis()
}