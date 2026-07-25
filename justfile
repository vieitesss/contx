executable := "contx"

default:
    just -l

build:
    @cargo build

run: build
    @echo
    RUST_LOG=debug ./target/debug/{{executable}}

rund: build
    @echo
    TUI_DEBUG=1 RUST_LOG=debug ./target/debug/{{executable}}
