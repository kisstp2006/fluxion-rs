#!/usr/bin/env bash
# Fluxion-RS build helper (bash / Git Bash / WSL / Linux / macOS)
# Usage:
#   ./build.sh              - release build of the whole workspace
#   ./build.sh debug        - debug build of the whole workspace
#   ./build.sh run          - release build + run the editor
#   ./build.sh run debug    - debug build + run the editor
#   ./build.sh check        - cargo check (fast, no codegen)
#   ./build.sh clean        - cargo clean
#   ./build.sh test         - cargo test (workspace)

set -e

MODE="${1:-release}"
EXTRA="${2:-}"

case "$MODE" in
    release|"")
        echo "[fluxion] cargo build --workspace --release"
        cargo build --workspace --release
        ;;
    debug)
        echo "[fluxion] cargo build --workspace"
        cargo build --workspace
        ;;
    check)
        echo "[fluxion] cargo check --workspace"
        cargo check --workspace
        ;;
    clean)
        echo "[fluxion] cargo clean"
        cargo clean
        ;;
    test)
        echo "[fluxion] cargo test --workspace"
        cargo test --workspace
        ;;
    run)
        if [ "$EXTRA" = "debug" ]; then
            echo "[fluxion] cargo run -p fluxion-editor"
            cargo run -p fluxion-editor
        else
            echo "[fluxion] cargo run -p fluxion-editor --release"
            cargo run -p fluxion-editor --release
        fi
        ;;
    *)
        echo "Unknown argument: $MODE"
        echo
        echo "Usage: ./build.sh [release|debug|run|check|clean|test] [debug]"
        exit 1
        ;;
esac
