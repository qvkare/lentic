# dstack's guest-agent simulator (github.com/Dstack-TEE/dstack, Apache-2.0), built from a pinned
# release for scripts/dstack-sim.sh. It serves the guest agent's API (GetKey, GetQuote, Info) on
# TCP :8090 instead of /var/run/dstack.sock, derives keys from fixed fixture app keys and answers
# GetQuote with a TDX quote captured from a real dstack CVM, its REPORTDATA patched to the
# request. Its attestation crates are Linux-only, hence the container on macOS.
FROM rust:1.96-bookworm AS build
ARG DSTACK_REF=v0.6.0
ARG DSTACK_COMMIT=4699c48ea3a7e568dff4f8792421299f27d50758
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
RUN git clone --depth 1 --branch "$DSTACK_REF" https://github.com/Dstack-TEE/dstack /dstack \
    && test "$(git -C /dstack rev-parse HEAD)" = "$DSTACK_COMMIT"
WORKDIR /dstack
RUN cargo build --locked --release --manifest-path dstack/Cargo.toml -p dstack-guest-agent-simulator

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /sim
COPY --from=build /dstack/sdk/simulator/appkeys.json /dstack/sdk/simulator/app-compose.json \
    /dstack/sdk/simulator/sys-config.json /dstack/sdk/simulator/attestation.bin ./
COPY --from=build /dstack/dstack/target/release/dstack-simulator ./
COPY dstack.toml ./
EXPOSE 8090
CMD ["./dstack-simulator", "-c", "dstack.toml"]
