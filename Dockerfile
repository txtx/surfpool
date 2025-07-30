FROM rust:bullseye

# Update and install dependencies
RUN apt-get update && \
    apt-get install -y pkg-config libudev-dev libssl-dev git libclang-dev clang && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Clone and install surfpool from source
COPY . ./surfpool

RUN cargo install --path ./surfpool/crates/cli --locked --force

# Copy the surfpool manifest file
COPY Surfpool.toml ./Surfpool.toml

# Expose common Solana ports
EXPOSE 8899 8900 8080

# Set the command to run surfpool
CMD /usr/local/cargo/bin/surfpool start --host 0.0.0.0 --debug --no-tui
