#!/bin/bash
cargo build --release
for file in tests/*.py; do
    ./package/pyapp $file
done
