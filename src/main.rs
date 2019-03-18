extern crate futures;
extern crate tokio;
extern crate trust_dns_resolver;

use bytes::{Bytes, BytesMut, IntoBuf};
use futures::{future, Async, Future, Poll, Stream};
use std::io::Cursor;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::{env, process};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::timer::Delay;
use trust_dns_resolver::{system_conf, AsyncResolver};

fn main() {
    let mut args = env::args();
    let _ = args.next().expect("must have at least a program name");
    match args.next() {
        Some(ref n) if n == "client" => client(args),
        Some(ref n) if n == "server" => server(args),
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

fn server(mut args: env::Args) {
    let port = match args.next() {
        Some(p) => p.parse::<u16>().expect("port must be valid"),
        None => usage(),
    };
    if args.next().is_some() {
        usage();
    }
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    tokio::run(futures::lazy(move || {
        tokio::net::TcpListener::bind(&addr)
            .expect("must bind")
            .incoming()
            .for_each(|socket| {
                println!("accepted");
                let _ = socket.set_nodelay(true);
                tokio::spawn(ServeEcho(
                    socket,
                    Some(ServeEchoInner::Read(BytesMut::with_capacity(1024 * 16))),
                ));
                Ok(())
            })
            .map_err(|e| panic!("listener failed: {}", e))
    }));
}

fn client(mut args: env::Args) {
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

    let (resolver, daemon) = {
        let (c, mut o) = system_conf::read_system_conf().expect("DNS configuration must be valid");
        o.cache_size = 0;
        AsyncResolver::new(c, o)
    };

    tokio::run(futures::lazy(move || {
        tokio::spawn(daemon);

        let mut t0 = Some(Instant::now());
        future::loop_fn(host, move |host| {
            resolver
                .lookup_ip(host.as_str())
                .map_err(|e| eprintln!("failed to lookup ip: {}", e))
                .and_then(move |ips| {
                    let ip = ips.iter().next().expect("must resolve to at least one IP");
                    println!("resolved host: {} => {}", host, ip);
                    let addr = SocketAddr::from((ip, port));
                    tokio::net::TcpStream::connect(&addr).then(move |s| {
                        let f = match s {
                            Err(e) => {
                                eprintln!("failed to connect: {}", e);
                                future::Either::A(future::ok(()))
                            }
                            Ok(socket) => {
                                match t0.take().map(|t| t.elapsed()) {
                                    None => println!("connected to {} at {}", host, addr),
                                    Some(d) => println!(
                                        "connected to {} at {} after {}ms",
                                        host,
                                        addr,
                                        d.as_secs() + u64::from(d.subsec_millis()),
                                    ),
                                }
                                let buf = Cursor::new(Bytes::from("heya"));
                                future::Either::B(
                                    DriveEcho(socket, Some(DriveEchoInner::Write(buf))).map(
                                        |buf| {
                                            println!(
                                                "read {:?}",
                                                ::std::str::from_utf8(buf.as_ref()).unwrap()
                                            );
                                        },
                                    ),
                                )
                            }
                        };

                        f.then(move |_| {
                            Delay::new(Instant::now() + Duration::from_millis(100))
                                .map_err(|_| panic!("timer failed"))
                                .map(|_| future::Loop::Continue(host))
                        })
                    })
                })
        })
    }));
}

// === Server ===

struct ServeEcho(tokio::net::TcpStream, Option<ServeEchoInner>);

enum ServeEchoInner {
    Read(BytesMut),
    Write(Cursor<Bytes>),
}

impl Future for ServeEcho {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.1.take().expect("polled after complete") {
                ServeEchoInner::Read(mut buf) => {
                    match self
                        .0
                        .read_buf(&mut buf)
                        .map_err(|e| eprintln!("read failed: {}", e))?
                    {
                        Async::NotReady => {
                            self.1 = Some(ServeEchoInner::Read(buf));
                            return Ok(Async::NotReady);
                        }
                        Async::Ready(sz) => {
                            println!("read {}B", sz);
                            self.1 = Some(ServeEchoInner::Write(buf.freeze().into_buf()));
                        }
                    }
                }
                ServeEchoInner::Write(mut buf) => {
                    match self
                        .0
                        .write_buf(&mut buf)
                        .map_err(|e| eprintln!("read failed: {}", e))?
                    {
                        Async::NotReady => {
                            self.1 = Some(ServeEchoInner::Write(buf));
                            return Ok(Async::NotReady);
                        }
                        Async::Ready(sz) => {
                            println!("wrote {}B", sz);
                            return Ok(Async::Ready(()));
                        }
                    }
                }
            }
        }
    }
}

// === Client ===

struct DriveEcho(tokio::net::TcpStream, Option<DriveEchoInner>);

enum DriveEchoInner {
    Write(Cursor<Bytes>),
    Read(BytesMut),
}

impl Future for DriveEcho {
    type Item = Bytes;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.1.take().expect("polled after complete") {
                DriveEchoInner::Write(mut buf) => {
                    match self
                        .0
                        .write_buf(&mut buf)
                        .map_err(|e| eprintln!("read failed: {}", e))?
                    {
                        Async::NotReady => {
                            self.1 = Some(DriveEchoInner::Write(buf));
                            return Ok(Async::NotReady);
                        }
                        Async::Ready(sz) => {
                            println!("wrote {}B", sz);
                            self.1 = Some(DriveEchoInner::Read(BytesMut::with_capacity(8 * 1024)));
                        }
                    }
                }
                DriveEchoInner::Read(mut buf) => {
                    match self
                        .0
                        .read_buf(&mut buf)
                        .map_err(|e| eprintln!("read failed: {}", e))?
                    {
                        Async::NotReady => {
                            self.1 = Some(DriveEchoInner::Read(buf));
                            return Ok(Async::NotReady);
                        }
                        Async::Ready(sz) => {
                            println!("read {}B", sz);
                            return Ok(Async::Ready(buf.freeze()));
                        }
                    }
                }
            }
        }
    }
}
