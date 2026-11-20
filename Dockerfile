# syntax=docker/dockerfile:1
# Build: docker build -t nirog-api .
# Runs as a non-root user on a distroless base (no shell, no package manager).

FROM rust:1-trixie AS build
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p nirog-server -p nirog-importer \
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
