#![deny(warnings, rust_2018_idioms)]

use bytes::BytesMut;
use clap::Parser;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    signal::{
        ctrl_c,
        unix::{signal, SignalKind},
    },
    time,
};
use tracing::{debug, error, info, info_span, warn, Instrument};

#[derive(clap::Parser)]
struct TcpEcho {
    #[clap(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Subcommand)]
enum Cmd {
    Client {
        #[clap(long, short, default_value = "1")]
        concurrency: usize,

        #[clap(long, short = 'M', default_value = "1")]
        messages_per_connection: usize,

        #[clap(long, short = 'R')]
        reverse: bool,

        #[clap(long, short, default_value = "PING\n")]
        message: String,

        targets: Vec<Target>,
    },
    Server {
        #[clap(long, short = 'R')]
        reverse: bool,

        #[clap(long, short, default_value = "PONG\n")]
        message: String,

        #[clap(long, short, default_value = "4444")]
        port: u16,
    },
}

const BUFSIZE: usize = 65 * 1024;

#[derive(Clone)]
enum Target {
    Dns(String, u16),
    Addr(SocketAddr),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + 'static>> {
    tracing_subscriber::fmt::init();
    match TcpEcho::parse().cmd {
        Cmd::Client {
            concurrency,
            messages_per_connection,
            reverse,
            message,
            targets,
        } => {
            if targets.is_empty() {
                return Err(InvalidTarget::Empty.into());
            }
            let rng = SmallRng::from_entropy();
            let msg: Arc<str> = message.into();
            for id in 0..concurrency.max(1) {
                tokio::spawn(
                    client(
                        targets.clone(),
                        messages_per_connection,
                        reverse,
                        msg.clone(),
                        rng.clone(),
                    )
                    .instrument(info_span!("worker", %id)),
                );
            }
        }
        Cmd::Server {
            reverse,
            message,
            port,
        } => {
            tokio::spawn(server(port, reverse, message.into()));
        }
    }

    let mut term = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = term.recv() => {}
        res = ctrl_c() => res?,
    }

    Ok(())
}

async fn server(port: u16, reverse: bool, message: Arc<str>) {
    let listen = match TcpListener::bind(SocketAddr::new(
        std::net::Ipv6Addr::UNSPECIFIED.into(),
        port,
    ))
    .await
    {
        Ok(lis) => lis,
        Err(error) => {
            warn!(%error, "failed to bind IPv6 socket");
            match TcpListener::bind(SocketAddr::new(
                std::net::Ipv6Addr::UNSPECIFIED.into(),
                port,
            ))
            .await
            {
                Ok(lis) => lis,
                Err(error) => {
                    error!(%error, "failed to bind IPv4 socket");
                    return;
                }
            }
        }
    };

    loop {
        let (socket, client) = match listen.accept().await {
            Ok(conn) => conn,
            Err(error) => {
                warn!(%error, "Failed to accept connection");
                continue;
            }
        };
        let server_addr = match socket.local_addr() {
            Ok(a) => a,
            Err(error) => {
                warn!(%error, "Failed to obtain local socket address");
                continue;
            }
        };
        if let Err(error) = socket.set_nodelay(true) {
            warn!(%error, "Failed to set TCP_NODELAY");
            continue;
        }
        let msg = message.clone();
        tokio::spawn(
            async move {
                if let Err(error) = serve_conn(socket, &*msg, reverse).await {
                    warn!(%error, "Connection failed");
                }
            }
            .instrument(info_span!("conn", server.addr = %server_addr, client.addr = %client)),
        );
    }
}

async fn serve_conn(mut socket: TcpStream, msg: &str, reverse: bool) -> std::io::Result<()> {
    info!("Accepted");
    let mut n = 0;
    let mut buf = BytesMut::with_capacity(BUFSIZE);
    loop {
        let sz = if reverse {
            write_read(&mut socket, &mut buf, msg).await?
        } else {
            read_write(&mut socket, &mut buf, msg).await?
        };
        if sz == 0 {
            debug!("Closed");
            return Ok(());
        }
        n += 1;
        debug!(%n);
        buf.clear()
    }
}

async fn client(
    targets: Vec<Target>,
    limit: usize,
    reverse: bool,
    message: Arc<str>,
    mut rng: SmallRng,
) {
    const BACKOFF: time::Duration = time::Duration::from_secs(1);

    let mut buf = BytesMut::with_capacity(BUFSIZE);
    loop {
        let target = &targets[rng.gen_range(0..targets.len())];
        let span = info_span!("conn", server.addr = %target);

        debug!(parent: &span, "Connecting");
        let socket = match target.connect().instrument(span.clone()).await {
            Ok(socket) => socket,
            Err(error) => {
                warn!(parent: &span, %error, "Connection failed");
                time::sleep(BACKOFF).await;
                continue;
            }
        };
        let client_addr = match socket.local_addr() {
            Ok(a) => a,
            Err(error) => {
                warn!(parent: &span, %error, "Failed to obtain local socket address");
                time::sleep(BACKOFF).await;
                continue;
            }
        };
        let span = info_span!("conn", server.addr = %target, client.addr = %client_addr);
        debug!(parent: &span, "Connected");

        if let Err(error) = target_client(socket, &mut buf, limit, reverse, &*message)
            .instrument(span.clone())
            .await
        {
            warn!(parent: &span, %error, "Connection closed");
            time::sleep(BACKOFF).await;
        }
    }
}

async fn target_client(
    mut socket: TcpStream,
    buf: &mut BytesMut,
    limit: usize,
    reverse: bool,
    message: &str,
) -> std::io::Result<()> {
    let mut n = 0;
    loop {
        n += 1;
        debug!(%n);

        buf.clear();
        let sz = if reverse {
            read_write(&mut socket, buf, message).await?
        } else {
            write_read(&mut socket, buf, message).await?
        };
        if sz == 0 {
            debug!("Closed");
            return Ok(());
        }

        if n == limit {
            debug!("Completed");
            return Ok(());
        }
        tokio::task::yield_now().await;
    }
}

/// Sends a message and reads a response.
async fn write_read(
    socket: &mut TcpStream,
    buf: &mut BytesMut,
    msg: &str,
) -> std::io::Result<usize> {
    socket.write_all(msg.as_bytes()).await?;
    socket.flush().await?;
    debug!(write.sz = %msg.len(), write.msg = ?msg);

    let sz = socket.read_buf(buf).await?;
    debug!(read.sz = %sz, read.msg = ?std::str::from_utf8(&buf[..sz]).unwrap_or("<invalid>"));

    Ok(sz)
}

/// Reads a message and then echoes it back.
async fn read_write(
    socket: &mut TcpStream,
    buf: &mut BytesMut,
    msg: &str,
) -> std::io::Result<usize> {
    let sz = socket.read_buf(buf).await?;
    debug!(read.sz = %sz, read.msg = ?std::str::from_utf8(&buf[..sz]).unwrap_or("<invalid>"));
    if sz == 0 {
        return Ok(0);
    }

    socket.write_all(msg.as_bytes()).await?;
    debug!(write.sz = %msg.len(), write.msg = ?msg);
    socket.flush().await?;

    Ok(sz)
}

// === impl Target ===

impl Target {
    async fn connect(&self) -> std::io::Result<TcpStream> {
        match self {
            Self::Addr(addr) => TcpStream::connect(addr).await,
            Self::Dns(ref host, port) => TcpStream::connect((host.as_str(), *port)).await,
        }
    }
}

impl std::str::FromStr for Target {
    type Err = InvalidTarget;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uri = http::Uri::from_str(s)?;
        match uri.scheme_str() {
            Some("tcp") | None => {}
            Some(s) => return Err(InvalidTarget::Scheme(s.to_string())),
        }

        let port = uri.port_u16().unwrap_or(4444);
        let host = uri
            .host()
            .ok_or(InvalidTarget::MissingHost)?
            .trim_start_matches('[')
            .trim_end_matches(']');
        let target = match host.parse::<std::net::IpAddr>() {
            Ok(ip) => Target::Addr(SocketAddr::new(ip, port)),
            Err(_) => Target::Dns(host.to_string(), port),
        };
        Ok(target)
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Addr(addr) => write!(f, "{}", addr),
            Self::Dns(h, p) => write!(f, "{}:{}", h, p),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum InvalidTarget {
    #[error("invalid scheme: {0}")]
    Scheme(String),
    #[error("missing host")]
    MissingHost,
    #[error("invalid uri: {0}")]
    Uri(#[from] http::uri::InvalidUri),
    #[error("no targets specified")]
    Empty,
}
