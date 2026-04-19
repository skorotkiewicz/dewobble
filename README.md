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

**Normal mode** (quiet, only shows blocked bounces):
```bash
./target/release/dewobble
```

**Verbose mode** (shows all clicks and mouse movements):
```bash
./target/release/dewobble --verbose
```

**X11 or permission errors:**
```bash
sudo ./target/release/dewobble
```

> Note: On Wayland, running with `sudo` often fails due to session permissions. Try without sudo first.

## Troubleshooting

**Permission denied:** Add your user to the `input` group and log out/back in:
```bash
sudo usermod -a -G input $USER
```

## Tuning

Edit `src/main.rs`:

```rust
const DEBOUNCE_MS: u64 = 100;        // Increase for bouncier switches
const MOVEMENT_THRESHOLD: f64 = 3.0;  // Increase for shakier hands
```

Then `cargo build --release` again.

## License

MIT
