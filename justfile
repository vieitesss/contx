executable := "contx"

default:
    just -l

build:
    @cargo build -r

run: build
    @echo
    RUST_LOG=debug ./target/release/{{executable}}

rund: build
    @echo
    TUI_DEBUG=1 RUST_LOG=debug ./target/release/{{executable}}
