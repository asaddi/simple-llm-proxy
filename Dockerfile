FROM rust:alpine AS build

WORKDIR /src
COPY . .

RUN \
  --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
  --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
  cargo build --locked --release

FROM alpine:latest

COPY --from=build /src/target/release/simple-llm-proxy /usr/local/bin/

ENV LLM_PROXY_BASE_URL=http://localhost:8080/v1
ENV LLM_PROXY_API_KEY=nokey

EXPOSE 3000

ENTRYPOINT ["/usr/local/bin/simple-llm-proxy", "--host", "0.0.0.0"]
CMD ["--base-url-env", "LLM_PROXY_BASE_URL", "--api-key-env", "LLM_PROXY_API_KEY"]
