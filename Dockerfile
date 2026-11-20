# syntax=docker/dockerfile:1
# SPDX-License-Identifier: Apache-2.0
#
# LENS-1 image: a static musl binary on distroless. No variant contains model
# weights. LENS_FEATURES selects the variant; nothing else is accepted:
#
#   ""             Reference image (default): the deterministic reference
#                  backend only, which is not a retrieval model. No model
#                  runtime, tokenizer, HTTP client or TLS stack.
#   "qwen3,fetch"  Model image: the Candle CPU backend and the fetcher. At
#                  startup it downloads the pinned bundles into a writable
#                  cache volume at /var/cache/lens, only from the fixed
#                  Hugging Face repository iokaio/munarium-lens-models, or
#                  from the fixed GitHub release backup if that fails.
#   "qwen3"        Offline model image without an HTTP client: mount the
#                  bundles read-only.
#
# Both stages are disabled and the listener is loopback-only unless the
# operator configures them at run time. Without bundle settings no variant
# makes an outbound connection. See docs/guides/local-development.md.
#
# The build fetches the locked crates. Both base images are pinned by digest.
# Update a digest deliberately and record it in the changelog.

FROM rust:1.98.1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS builder
# Use the toolchain already in the base image instead of rust-toolchain.toml,
# which would also download the contributor lint components.
ENV RUSTUP_TOOLCHAIN=1.98.1
RUN apk add --no-cache musl-dev
# Empty, `qwen3,fetch` or `qwen3`. Nothing else is accepted.
ARG LENS_FEATURES=""
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN case "$LENS_FEATURES" in \
      "" | qwen3 | qwen3,fetch) ;; \
      *) echo "LENS_FEATURES must be empty, qwen3,fetch or qwen3" >&2; exit 1 ;; \
    esac \
 && cargo build --locked --release ${LENS_FEATURES:+--features "$LENS_FEATURES"} \
 && mkdir -p /out/cache \
 && cp target/release/munarium-lens /out/munarium-lens

FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
ARG SOURCE_REVISION="unknown"
ARG LENS_FEATURES=""
LABEL org.opencontainers.image.source="https://github.com/iokaio/munarium-lens" \
      org.opencontainers.image.revision=$SOURCE_REVISION \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.title="Munarium Lens (${LENS_FEATURES:-reference backend})" \
      org.opencontainers.image.description="Munarium Lens, cargo features: ${LENS_FEATURES:-none}. The default build serves only the deterministic reference backend, which is not a retrieval model. No image contains model weights. The fetch feature downloads pinned bundles only from the fixed Munarium Hugging Face repository or its GitHub release backup."
COPY --from=builder /out/munarium-lens /munarium-lens
# An empty cache directory owned by the runtime user, so a named volume
# mounted here starts writable. With a read-only root filesystem, mount a
# volume (emptyDir or a PVC) here before setting LENS_MODEL_CACHE_DIR.
COPY --from=builder --chown=65532:65532 /out/cache /var/cache/lens
COPY LICENSE NOTICE THIRD_PARTY_NOTICES.md /usr/share/licenses/munarium-lens/
USER nonroot
EXPOSE 8787
ENTRYPOINT ["/munarium-lens"]
