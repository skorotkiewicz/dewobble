# dewobble

Filter out phantom clicks and mouse jitter.

## What it does

- **Debounces clicks**: Ignores button bounces within 100ms (fixing worn-out mouse switches)
- **Smooths movement**: Absorbs tiny movements under 3px (helping shaky hands or high-DPI mice)

## Install

```bash
git clone <repo>
cd dewobble
cargo build --release
```

## Run

```bash
sudo ./target/release/dewobble
```

> `sudo` required for raw input access on Linux.

## Tuning

Edit `src/main.rs`:

```rust
const DEBOUNCE_MS: u64 = 100;        // Increase for bouncier switches
const MOVEMENT_THRESHOLD: f64 = 3.0;  // Increase for shakier hands
```

Then `cargo build --release` again.

## License

MIT
