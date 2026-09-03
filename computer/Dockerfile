FROM rust:1-alpine AS build

RUN apk add --no-cache build-base cmake perl
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build -p toad-computer --release

FROM jlesage/baseimage-gui:alpine-3.22-v4

RUN apk add --no-cache \
    at-spi2-core \
    chromium \
    dbus \
    font-noto \
    font-noto-emoji \
    ttf-dejavu \
    xclip \
    xdotool \
    && mkdir -p /home/agent \
    && chown 1000:1000 /home/agent

COPY --from=build /src/target/release/toad-computer /usr/bin/toad-computer
COPY computer/startapp.sh /startapp.sh

RUN chmod 0755 /startapp.sh /usr/bin/toad-computer

ENV ACCESSIBILITY_ENABLED=1 \
    APP_NAME="Toad Computer" \
    TOAD_COMPUTER_ADDR=0.0.0.0:8787 \
    TOAD_COMPUTER_HOME=/home/agent

EXPOSE 8787 5800 5900
