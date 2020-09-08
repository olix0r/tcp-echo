#![deny(warnings, rust_2018_idioms)]

use bytes::BytesMut;
use futures::prelude::*;
use std::{env, net::SocketAddr, process};
use tokio::{
    net::{TcpListener, TcpStream},
    prelude::*,
    time,
};
use trust_dns_resolver::{system_conf, AsyncResolver, TokioAsyncResolver};

#[tokio::main]
async fn main() {
    let mut args = env::args();
    let _ = args.next().expect("must have at least a program name");
    match args.next() {
        Some(ref n) if n == "client" => client(args).await,
        Some(ref n) if n == "server" => server(args).await,
        a => {
            eprintln!("need at least one argument: {:?}", a);
            usage();
        }
    }
}

fn usage() -> ! {
    eprintln!("client <host> <port>");
    eprintln!("server <port>");
    process::exit(64);
}

async fn server(mut args: env::Args) {
    let port = match args.next() {
        Some(p) => p.parse::<u16>().expect("port must be valid"),
        None => usage(),
    };
    if args.next().is_some() {
        usage();
    }
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let mut server = TcpListener::bind(&addr).await.expect("must bind");
    let mut incoming = server.incoming();

    while let Some(socket) = incoming.try_next().await.expect("Server failed") {
        let peer = socket.peer_addr().unwrap();
        let _ = socket.set_nodelay(true);
        tokio::spawn(async move {
            println!("Processing connection from {}", peer);
            match serve_conn(socket).await {
                Ok(sz) => println!("Echoed {}B from {}", sz, peer),
                Err(e) => eprintln!("Failed to serve connection from {}: {}", peer, e),
            }
        });
    }
}

async fn client(mut args: env::Args) {
    let host = match args.next() {
        Some(h) => h,
        None => usage(),
    };
    let port = match args.next() {
        Some(p) => p.parse::<u16>().expect("port must be valid"),
        None => usage(),
    };
    if args.next().is_some() {
        usage();
    }

    let mut resolver = {
        let (c, mut o) = system_conf::read_system_conf().expect("DNS configuration must be valid");
        o.cache_size = 0;
        AsyncResolver::tokio(c, o)
            .await
            .expect("DNS resolver must be valid")
    };

    const BASE_SLEEP: time::Duration = time::Duration::from_millis(100);
    const MESSAGE: &str = "heya!

    how's it going?
    ";
    let mut buf = BytesMut::with_capacity(MESSAGE.len());
    let mut sleep = BASE_SLEEP;
    loop {
        sleep = match client_send(&mut resolver, &host, port, MESSAGE, &mut buf).await {
            Err(e) => {
                eprintln!("Failed to send message: {}", e);
                sleep * 2
            }
            Ok(sz) => {
                println!(
                    "Read {}B: {:?}",
                    sz,
                    ::std::str::from_utf8(buf.as_ref()).unwrap()
                );
                BASE_SLEEP
            }
        };
        buf.clear();

        time::delay_for(sleep).await;
    }
}

async fn client_send(
    resolver: &mut TokioAsyncResolver,
    host: &str,
    port: u16,
    msg: &str,
    buf: &mut BytesMut,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let ips = resolver.lookup_ip(host).await?;
    let ip = ips.iter().next().expect("must resolve to at least one IP");
    let addr = SocketAddr::from((ip, port));

    let mut socket = TcpStream::connect(&addr).await?;
    println!("Connected to {} at {}", host, addr);
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
