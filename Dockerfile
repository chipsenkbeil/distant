FROM rust:alpine AS builder
# make & perl needed for openssl vendor build
# musl-dev needed, otherwise crti.o is missing
RUN apk add --no-cache make musl-dev perl
WORKDIR /usr/src/distant
COPY . .
# NOTE: For musl clients, there is a bug with wezterm-ssh's libssh-rs that
#       prevents us from linking to it; so, we have to exclude it for musl
RUN cargo build --release --no-default-features --features ssh2

FROM alpine:3
COPY --from=builder /usr/src/distant/target/release/distant /usr/local/bin/distant
ENTRYPOINT ["/usr/local/bin/distant"]
