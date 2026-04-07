FROM debian:bookworm-slim

# Install dependencies
RUN apt-get update && apt-get install -y curl sqlite3 && rm -rf /var/lib/apt/lists/*

# Run the official installer (installs to /root/.cargo/bin)
RUN curl -fsSL https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh

# Add cargo bin folder to PATH
ENV PATH="/root/.cargo/bin:$PATH"

# Create config directory with empty DB so onboarding is skipped
RUN mkdir -p /root/.ironclaw \
    && touch /root/.ironclaw/ironclaw.db

EXPOSE 3000

CMD ["ironclaw", "--no-onboard"]
