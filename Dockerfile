# syntax=docker/dockerfile:experimental

ARG RUST_IMAGE=rust:1.45.2
ARG RUNTIME_IMAGE=debian:buster-20200803-slim

FROM $RUST_IMAGE as build
WORKDIR /usr/src/init-net-test
COPY . .
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=rust:1.45.2,source=/usr/local/cargo,target=/usr/local/cargo \
    cargo build --locked --release &&  mv target/release/init-net-test /tmp

FROM $RUNTIME_IMAGE as runtime
COPY --from=build /tmp/init-net-test /usr/bin/init-net-test
ENTRYPOINT ["/usr/bin/init-net-test"]
