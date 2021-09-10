ARG RUST_IMAGE=rust:1.55.0

FROM $RUST_IMAGE as build
ARG TARGETARCH
WORKDIR /usr/src/tcp-echo
COPY . .
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=rust:1.55.0,source=/usr/local/cargo,target=/usr/local/cargo \
    target=$(rustup show | sed -n 's/^Default host: \(.*\)/\1/p' | sed 's/-gnu$/-musl/') ; \
    rustup target add ${target} && \
    cargo build --locked --release --target=$target && \
    mv target/${target}/release/tcp-echo /tmp

FROM docker.io/curlimages/curl:7.75.0 as await
ARG LINKERD_AWAIT_VERSION=v0.2.1
ARG TARGETARCH
RUN curl -fvsLo /tmp/linkerd-await https://github.com/olix0r/linkerd-await/releases/download/release/${LINKERD_AWAIT_VERSION}/linkerd-await-${LINKERD_AWAIT_VERSION}-${TARGETARCH} && chmod +x /tmp/linkerd-await

FROM scratch
COPY --from=await /tmp/linkerd-await /linkerd-await
COPY --from=build /tmp/tcp-echo /tcp-echo
ENTRYPOINT ["/linkerd-await", "--", "/tcp-echo"]
