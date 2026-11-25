# syntax=docker/dockerfile:1
# Build: docker build -t nirog-api .
# Runs as a non-root user on a distroless base (no shell, no package manager).
#
# Layered with cargo-chef so caches can reuse work: the third-party crates (most of the build time)
# are compiled in their own layer, rebuilt only when Cargo.toml/Cargo.lock change; a normal change
# rebuilds only Nirog's own crates.

FROM rust:1-trixie AS chef
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install
RUN cargo install cargo-chef --version 0.1.78 --locked

# The dependency list only: changes when Cargo.toml or Cargo.lock does.
FROM chef AS plan
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
COPY --from=plan /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json -p nirog-server -p nirog-importer
COPY . .
RUN cargo build --release --locked -p nirog-server -p nirog-importer \
 && mkdir -p /out \
 && cp target/release/nirog-server target/release/nirog-importer /out/

FROM gcr.io/distroless/cc-debian13:nonroot
WORKDIR /app
COPY --from=build /out/nirog-server /out/nirog-importer /app/
COPY data/janaushadhi-products.csv /app/data/
USER nonroot:nonroot
ENV PORT=8080 RUST_LOG=info
EXPOSE 8080
ENTRYPOINT ["/app/nirog-server"]
