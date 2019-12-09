ARG RUST_IMAGE=rust:1.39.0
ARG RUNTIME_IMAGE=debian:stretch-20190204-slim

FROM $RUST_IMAGE as build
WORKDIR /usr/src/init-net-test
RUN mkdir -p src && touch src/lib.rs
COPY Cargo.toml Cargo.lock ./
RUN cargo fetch --locked
COPY src src
RUN cargo build --frozen --release

FROM $RUNTIME_IMAGE as runtime
COPY --from=build /usr/src/init-net-test/target/release/init-net-test /init-net-test
ENTRYPOINT ["/usr/bin/init-net-test"]
