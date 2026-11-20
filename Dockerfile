# syntax=docker/dockerfile:1
# SPDX-License-Identifier: Apache-2.0
#
# LENS-1 reference image: a static musl binary on distroless. It contains the
# deterministic reference backend only, which is not a retrieval model, and no
# model runtime, weights or tokenizer. Both stages are disabled and the
# listener is loopback-only unless the operator configures them at run time.
# See docs/guides/local-development.md for the container walkthrough.
#
# The build fetches the locked crates; the running image needs no network.
# Both base images are pinned by digest. Update a digest deliberately and
# record it in the changelog.

FROM rust:1.98.1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS builder
# Use the toolchain already in the base image instead of rust-toolchain.toml,
# which would also download the contributor lint components.
ENV RUSTUP_TOOLCHAIN=1.98.1
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release \
 && mkdir -p /out \
 && cp target/release/munarium-lens /out/munarium-lens

FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
ARG SOURCE_REVISION="unknown"
LABEL org.opencontainers.image.source="https://github.com/iokaio/munarium-lens" \
      org.opencontainers.image.revision=$SOURCE_REVISION \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.title="Munarium Lens (reference backend)" \
      org.opencontainers.image.description="Munarium Lens with the deterministic reference backend only. Not a retrieval model."
COPY --from=builder /out/munarium-lens /munarium-lens
COPY LICENSE NOTICE THIRD_PARTY_NOTICES.md /usr/share/licenses/munarium-lens/
USER nonroot
EXPOSE 8787
ENTRYPOINT ["/munarium-lens"]
