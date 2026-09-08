executable := "contx"

default:
    just -l

build:
    @cargo build

run: build
    @echo
    RUST_LOG=debug ./target/debug/{{executable}} -c ./config.toml
