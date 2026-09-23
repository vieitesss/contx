executable := "contx"

default:
    just -l

build:
    @cargo build

run: build
    @echo
    RUST_LOG=debug ./target/debug/{{executable}} -c ./config.toml

install:
    rm ~/.local/bin/contx | true
    cargo build -r
    ln -sf ~/personal/contx/target/release/contx ~/.local/bin/contx
