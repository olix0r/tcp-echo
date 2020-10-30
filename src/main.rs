#![deny(warnings, rust_2018_idioms)]

use bytes::BytesMut;
use futures::prelude::*;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::net::SocketAddr;
use structopt::StructOpt;
use tokio::{
    net::{TcpListener, TcpStream},
    prelude::*,
    signal::unix::{signal, SignalKind},
    time,
};
use tracing::{debug, info, warn};

#[derive(StructOpt)]
enum TcpEcho {
    Client {
        #[structopt(long, short, env)]
        concurrency: usize,
        targets: Vec<Target>,
    },
    Server {
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
            targets,
        } => {
            if targets.is_empty() {
                return Err(InvalidTarget.into());
            }
            let rng = SmallRng::from_entropy();
            for _ in 0..concurrency.min(1) {
                tokio::spawn(client(targets.clone(), rng.clone()));
            }
        }
        TcpEcho::Server { port } => {
            tokio::spawn(server(port));
        }
    }

    signal(SignalKind::terminate())?.recv().await;

    Ok(())
}

async fn server(port: u16) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let server = TcpListener::bind(&addr).await.expect("must bind");
    loop {
        let (socket, peer) = server.accept().await.expect("Server failed");
        let _ = socket.set_nodelay(true);
        tokio::spawn(async move {
            info!("Processing connection from {}", peer);
            match serve_conn(socket).await {
                Ok(sz) => debug!("Echoed {}B from {}", sz, peer),
                Err(e) => warn!("Failed to serve connection from {}: {}", peer, e),
            }
        });
    }
}

async fn client(targets: Vec<Target>, mut rng: SmallRng) {
    const BACKOFF: time::Duration = time::Duration::from_secs(1);
    const MESSAGE: &str = "heya!

    how's it going?
    ";

    let mut buf = BytesMut::with_capacity(MESSAGE.len());
    loop {
        let i = rng.gen::<usize>();
        let Target(ref host, port) = targets[i % targets.len()];
        match client_send(host, port, MESSAGE, &mut buf).await {
            Err(e) => {
                warn!("Failed to send message: {}", e);
                time::sleep(BACKOFF).await;
            }
            Ok(sz) => {
                debug!(
                    "Read {}B: {:?}",
                    sz,
                    ::std::str::from_utf8(buf.as_ref()).unwrap()
                );
            }
        };
        buf.clear();
    }
}

async fn client_send(
    host: &str,
    port: u16,
    msg: &str,
    buf: &mut BytesMut,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let mut socket = TcpStream::connect((host, port)).await?;
    debug!("Connected to {}:{}", host, port);
    socket.write_all(msg.as_bytes()).await?;
    socket.read_buf(buf).err_into().await
}

async fn serve_conn(
    mut socket: TcpStream,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = [0u8; 1024 * 8];
    let sz = socket.read(&mut buf).await?;
    socket.write_all(&buf).await?;
    Ok(sz)
}

// === impl Target ===

impl std::str::FromStr for Target {
    type Err = Box<dyn std::error::Error + 'static>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uri = http::Uri::from_str(s)?;
        if let Some("tcp") | None = uri.scheme_str() {
            if let (Some(h), Some(p)) = (uri.host(), uri.port_u16()) {
                return Ok(Target(h.to_string(), p));
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
