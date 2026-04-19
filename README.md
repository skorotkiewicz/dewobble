# dewobble

Filter out phantom clicks and mouse jitter.

## What it does

- **Debounces clicks**: Ignores button bounces within 100ms (fixing worn-out mouse switches)
- **Debounces scroll**: Ignores scroll wheel bounces in opposite direction (fixing defective scroll wheels that scroll up when you scroll down)
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

**Hold mode** (absorb rapid clicks as held state instead of blocking):
```bash
./target/release/dewobble --hold
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

## Modes Explained

**BLOCK mode** (default): Rapid clicks within 100ms are suppressed entirely.

**HOLD mode** (`--hold`): Rapid clicks are converted to a "held" state - the button stays pressed until the debounce period (100ms) passes. This feels more like a clean mechanical switch.

## Troubleshooting

**Permission denied:** Add your user to the `input` group and log out/back in:
```bash
sudo usermod -a -G input $USER
```

## Tuning

Set via environment variables:

```bash
# Adjust debounce window (default: 100ms)
DEBOUNCE_MS=150 ./target/release/dewobble

# Adjust movement threshold (default: 3.0 pixels)
MOVEMENT_THRESHOLD=5.0 ./target/release/dewobble

# Adjust scroll debounce (default: 50ms) - increase for bouncier scroll wheels
SCROLL_DEBOUNCE_MS=100 ./target/release/dewobble

# Combined example
DEBOUNCE_MS=200 MOVEMENT_THRESHOLD=10.0 SCROLL_DEBOUNCE_MS=100 ./target/release/dewobble --hold
```

Or edit `src/main.rs` for permanent changes:

```rust
const DEFAULT_DEBOUNCE_MS: u64 = 100;        // Increase for bouncier switches
const DEFAULT_MOVEMENT_THRESHOLD: f64 = 3.0;  // Increase for shakier hands
const DEFAULT_SCROLL_DEBOUNCE_MS: u64 = 50;   // Increase for bouncier scroll wheels
```

Then `cargo build --release` again.

## License

MIT
