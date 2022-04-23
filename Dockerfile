FROM rust:1.60-slim-bullseye AS rust-build
RUN apt-get update -y && apt-get install -y wget gcc build-essential cmake
ADD . /code

# Uncomment for attaching a memry profiler
# RUN cd /code && \
#     wget https://github.com/koute/bytehound/releases/download/0.8.0/bytehound-x86_64-unknown-linux-gnu.tgz -q && \
#     tar xfv bytehound-x86_64-unknown-linux-gnu.tgz

RUN cd /code && cargo build --release

FROM debian:bullseye-slim
WORKDIR /home
COPY --from=rust-build /code/target/release/rensa .
COPY --from=rust-build /code/test/genesis.json .

# Uncomment for attaching a memry profiler
# COPY --from=rust-build /code/bytehound .
# COPY --from=rust-build /code/libbytehound.so .

EXPOSE 44668