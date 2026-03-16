# NeoBrowser — headless browser for AI agents
# Build: docker build -t neobrowser .
# Run:   docker run -it --rm -v ~/.neobrowser:/root/.neobrowser neobrowser mcp

FROM rust:1.82 AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    chromium \
    ca-certificates \
    fonts-liberation \
    libatk-bridge2.0-0 \
    libatk1.0-0 \
    libcups2 \
    libdbus-1-3 \
    libdrm2 \
    libgbm1 \
    libnss3 \
    libxcomposite1 \
    libxdamage1 \
    libxrandr2 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/neobrowser_rs /usr/local/bin/

ENV NEOBROWSER_HEADLESS=1

ENTRYPOINT ["neobrowser_rs"]
CMD ["mcp"]
