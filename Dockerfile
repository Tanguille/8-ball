# syntax=docker/dockerfile:1
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
ENV CARGO_TARGET_DIR=/src/target
RUN cargo build --release --locked && cp target/release/8ball /8ball

FROM gcr.io/distroless/static:nonroot
COPY --from=build /8ball /8ball
ENV EIGHTBALL_LISTEN=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/8ball"]
CMD ["serve"]
