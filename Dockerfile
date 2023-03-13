# Build Stage
FROM rust:1.68-alpine3.17 AS builder
RUN apk update && apk add --no-cache libressl-dev libc-dev

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
