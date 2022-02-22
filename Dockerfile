FROM rust:1.58-slim-bullseye AS rust-build
RUN apt-get update -y && apt-get install -y clang
ADD . /code
RUN cd /code && cargo build --release

FROM debian:bullseye-slim
WORKDIR /home
COPY --from=rust-build /code/target/release/rensa .
COPY --from=rust-build /code/test/genesis.json .

EXPOSE 44668