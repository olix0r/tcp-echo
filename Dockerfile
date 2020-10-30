# syntax=docker/dockerfile:experimental

ARG RUST_IMAGE=rust:1.45.2
ARG RUNTIME_IMAGE=debian:buster-20200803-slim

FROM $RUST_IMAGE as build
WORKDIR /usr/src/tcp-echo
COPY . .
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=rust:1.45.2,source=/usr/local/cargo,target=/usr/local/cargo \
    cargo build --locked --release &&  mv target/release/tcp-echo /tmp

FROM $RUNTIME_IMAGE as runtime
COPY --from=build /tmp/tcp-echo /usr/bin/tcp-echo
ENTRYPOINT ["/usr/bin/tcp-echo"]
