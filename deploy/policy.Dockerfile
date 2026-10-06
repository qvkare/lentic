# Policy signer image. In production it runs inside a dstack CVM, where the image digest is
# part of the compose hash extended into RTMR3, i.e. part of what AttestationVerifier pins.
# Build from the repo root:  docker build -f deploy/policy.Dockerfile -t lentic-policy .
FROM rust:1.96-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates crates
# lentic-base generates its bindings from these ABIs at compile time.
COPY contracts/abi contracts/abi
COPY contracts/test/vectors contracts/test/vectors
RUN cargo build --release --locked -p lentic-policy

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/lentic-policy /usr/local/bin/lentic-policy
COPY contracts/deployments /app/deployments
ENV LENTIC_LISTEN=0.0.0.0:8787
EXPOSE 8787
ENTRYPOINT ["lentic-policy"]
CMD ["serve"]
