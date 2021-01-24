# syntax=docker/dockerfile:experimental

ARG RUST_IMAGE=rust:1.49.0

FROM $RUST_IMAGE as build
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /usr/src/tcp-echo
COPY . .
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=rust:1.49.0,source=/usr/local/cargo,target=/usr/local/cargo \
    cargo build --locked --release --target=x86_64-unknown-linux-musl && \
    mv target/x86_64-unknown-linux-musl/release/tcp-echo /tmp

FROM scratch
COPY --from=build /tmp/tcp-echo /tcp-echo
ENTRYPOINT ["/tcp-echo"]
