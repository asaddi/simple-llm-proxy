FROM rust:alpine AS build

WORKDIR /src
COPY . .

RUN \
  --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
  --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
  cargo build --locked --release

FROM alpine:latest

COPY --from=build /src/target/release/simple-llm-proxy /usr/local/bin/

# Note: Mount the config at /config.yaml
EXPOSE 3000

USER nobody:nobody

ENTRYPOINT ["/usr/local/bin/simple-llm-proxy", "--host", "0.0.0.0"]
CMD ["--port", "3000"]
