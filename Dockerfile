# Build Stage
FROM rust:1.68-alpine3.17 AS builder
RUN apk update && apk add --no-cache libc-dev ca-certificates musl musl-dev openssl-dev openssl-libs-static

#RUN apk add --no-cache  sqlite-static  pkgconf git libpq-dev


# Set `SYSROOT` to a dummy path (default is /usr) because pkg-config-rs *always*
# links those located in that path dynamically but we want static linking, c.f.
# https://github.com/rust-lang/pkg-config-rs/blob/54325785816695df031cef3b26b6a9a203bbc01b/src/lib.rs#L613
ENV SYSROOT=/dummy
# The env var tells pkg-config-rs to statically link libpq.
ENV LIBPQ_STATIC=1

WORKDIR /usr/src/
RUN rustup target add x86_64-unknown-linux-musl

RUN USER=root cargo new main
WORKDIR /usr/src/main
COPY Cargo.toml Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/src/main/target \
    cargo build --release

COPY src ./src
#COPY templates ./templates

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/src/main/target \
    cargo install --target x86_64-unknown-linux-musl --path .

# create tmp folder for use in scratch
RUN mkdir /my_tmp
#RUN chown -R ${UID}:${UID} /my_tmp

# Bundle Stage
FROM scratch
ARG UID=10001
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
# Import the user and group files from the builder.
#COPY --from=builder /etc/passwd /etc/passwd
#COPY --from=builder /etc/group /etc/group
# create tmp directory
COPY --from=builder --chown=${UID}:${UID} /my_tmp /tmp
# Copy static executable.
COPY --from=builder --chown=${UID}:${UID} /usr/local/cargo/bin/sero main
ENV UID=$UID
# Use an unprivileged user.
USER ${UID}:${UID}
LABEL org.opencontainers.image.source=https://github.com/fluktuid/sero.rs

ENTRYPOINT ["./main"]
