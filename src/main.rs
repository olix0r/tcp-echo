#![deny(warnings, rust_2018_idioms)]

use bytes::BytesMut;
use futures::prelude::*;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::net::SocketAddr;
use structopt::StructOpt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    signal::{
        ctrl_c,
        unix::{signal, SignalKind},
    },
    time,
};
use tracing::{debug, info, warn};

const MESSAGE: &str = "heya!

how's it going?
";

#[derive(StructOpt)]
enum TcpEcho {
    Client {
        #[structopt(long, short, env, default_value = "1")]
        concurrency: usize,
        #[structopt(long, short = "R")]
        reverse: bool,
        targets: Vec<Target>,
    },
    Server {
        #[structopt(long, short = "R")]
        reverse: bool,
        port: u16,
    },
}

#[derive(Clone)]
struct Target(String, u16);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + 'static>> {
    tracing_subscriber::fmt::init();
    match TcpEcho::from_args() {
        TcpEcho::Client {
            concurrency,
            reverse,
            targets,
        } => {
            if targets.is_empty() {
                return Err(InvalidTarget.into());
            }
            let rng = SmallRng::from_entropy();
            for _ in 0..concurrency.min(1) {
                tokio::spawn(client(targets.clone(), reverse, rng.clone()));
            }
        }
        TcpEcho::Server { reverse, port } => {
            tokio::spawn(server(reverse, port));
        }
    }

    let mut term = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = term.recv() => {}
        res = ctrl_c() => res?,
    }

    Ok(())
}

async fn server(reverse: bool, port: u16) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let server = TcpListener::bind(&addr).await.expect("must bind");
    loop {
        let (mut socket, client) = server.accept().await.expect("Server failed");
        let _ = socket.set_nodelay(true);
        tokio::spawn(async move {
            info!(%client, reverse, "Accepted connection");
            let mut buf = BytesMut::with_capacity(1024);
            if reverse {
                match write_read(&mut socket, MESSAGE, &mut buf).await {
                    Err(error) => warn!(%client, %error, "Failed to send message"),
                    Ok(sz) => {
                        debug!(
                            %client,
                            "Read {}B: {:?}",
                            sz,
                            std::str::from_utf8(buf.as_ref()).unwrap()
                        );
                    }
                }
            } else {
                loop {
                    match read_write(&mut socket, &mut buf).await {
                        Ok(0) => {
                            debug!(%client, "Closed");
                            return;
                        }
                        Ok(sz) => {
                            debug!(%client, "Echoed {}B", sz);
                            buf.clear();
                        }
                        Err(error) => {
                            warn!(%client, %error);
                            return;
                        }
                    }
                }
            }
        });
    }
}

async fn client(targets: Vec<Target>, reverse: bool, mut rng: SmallRng) {
    const BACKOFF: time::Duration = time::Duration::from_secs(1);

    let mut buf = BytesMut::with_capacity(MESSAGE.len());
    loop {
        let i = rng.gen::<usize>();
        let Target(ref host, port) = targets[i % targets.len()];
        match TcpStream::connect((host.as_str(), port)).await {
            Err(error) => {
                warn!(%error, "Failed to connect");
                time::sleep(BACKOFF).await;
            }
            Ok(mut socket) => {
                debug!(reverse, "Connected to {}:{}", host, port);
                if reverse {
                    loop {
                        match read_write(&mut socket, &mut buf).await {
                            Ok(0) => {
                                debug!(%host, port, "Closed");
                                break;
                            }
                            Ok(sz) => {
                                debug!(%host, port, "Echoed {}B", sz);
                                buf.clear();
                            }
                            Err(error) => {
                                warn!(%host, port, %error);
                                break;
                            }
                        }
                    }
                } else {
                    match write_read(&mut socket, MESSAGE, &mut buf).await {
                        Err(error) => {
                            warn!(%error, "Failed to send message");
                            time::sleep(BACKOFF).await;
                        }
                        Ok(sz) => {
                            debug!(
                                %host,
                                port,
                                "Read {}B: {:?}",
                                sz,
                                std::str::from_utf8(buf.as_ref()).unwrap()
                            );
                        }
                    }
                }
            }
        };
        buf.clear();
    }
}

async fn write_read(
    socket: &mut TcpStream,
    msg: &str,
    buf: &mut BytesMut,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    socket.write_all(msg.as_bytes()).await?;
    socket.read_buf(buf).err_into().await
}

async fn read_write(
    socket: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let sz = socket.read_buf(buf).await?;
    let mut remaining = sz;
    while remaining != 0 {
        remaining -= socket.write_buf(buf).await?;
    }
    Ok(sz)
}

// === impl Target ===

impl std::str::FromStr for Target {
    type Err = Box<dyn std::error::Error + 'static>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uri = http::Uri::from_str(s)?;
        if let Some("tcp") | None = uri.scheme_str() {
            if let Some(h) = uri.host() {
                return Ok(Target(h.to_string(), uri.port_u16().unwrap_or(4444)));
            }
        }

        Err(InvalidTarget.into())
    }
}

#[derive(Debug)]
struct InvalidTarget;

impl std::fmt::Display for InvalidTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid target")
    }
}

impl std::error::Error for InvalidTarget {}
