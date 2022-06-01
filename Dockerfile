FROM alpine:3

# Install curl so we can download the distant binary
RUN apk add --no-cache curl

# 1. Specify the distant version
# 2. Download the MUSL artifact as alpine uses musl
# 3. Make the binary executable
ARG distant_version=0.16.3
ARG distant_url=https://github.com/chipsenkbeil/distant/releases/download/v${distant_version}/distant-linux64-musl
RUN curl -L ${distant_url} > /usr/local/bin/distant && chmod +x /usr/local/bin/distant

ENTRYPOINT ["/usr/local/bin/distant"]
