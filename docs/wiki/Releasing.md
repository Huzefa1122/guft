# Releasing

For maintainers.

1. `cargo update`, then `./ci.sh` (tests, clippy, `cargo audit`, `cargo deny`, UI build). Run the live Tor tests.
2. Update the version in `Cargo.toml` and `src-tauri/tauri.conf.json`; add release notes.
3. Build the release binary with paths scrubbed:

   ```sh
   (cd ui && pnpm build)
   export RUSTFLAGS="-C link-arg=-Wl,-z,relro -C link-arg=-Wl,-z,now -C link-arg=-Wl,-z,noexecstack \
     --remap-path-prefix=$HOME=/build/home --remap-path-prefix=$PWD=/build/guft"
   cargo build --release --locked -p guft-desktop --target-dir target/dist
   ```

4. Check the binary: `readelf -lW` shows `GNU_RELRO` and a non-executable `GNU_STACK`, `readelf -d` shows `BIND_NOW`, and `strings` finds no home-directory path.
5. Package `guft-<version>-linux-x86_64.tar.gz` (binary, `LICENSE`, `README.md`, `guft.desktop`, `guft.png`), write `SHA256SUMS`, sign it: `gpg --armor --detach-sign SHA256SUMS`.
6. Tag with a signed tag and create the GitHub release, marking it a **pre-release** while guft is unaudited.
7. Commits and release notes never carry tool or assistant attributions.
