use rdev::{Button, Event, EventType, listen};
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Minimum time between clicks to be considered separate (not bounce)
const DEBOUNCE_MS: u64 = 100;

// Minimum mouse movement in pixels to not be considered "jitter"
// Movements smaller than this will be absorbed
const MOVEMENT_THRESHOLD: f64 = 3.0;

// Convert button to a simple ID for tracking
fn button_id(button: &Button) -> &'static str {
    match button {
        Button::Left => "L",
        Button::Right => "R",
        Button::Middle => "M",
        Button::Unknown(n) => match n {
            1 => "Back",
            2 => "Forward",
            _ => "X",
        },
    }
}

fn main() {
    let verbose = env::args().any(|arg| arg == "--verbose" || arg == "-v");

    println!("dewobble started!");
    println!("Filtering clicks faster than {}ms", DEBOUNCE_MS);
    println!(
        "Filtering movements smaller than {} pixels",
        MOVEMENT_THRESHOLD
    );
    println!("Press Ctrl+C to exit\n");

    // Track last click time for each button
    let last_clicks: Arc<Mutex<HashMap<&'static str, Instant>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Track mouse position for jitter filtering
    // We store the "reference" position - the last position we considered "significant"
    let last_position: Arc<Mutex<Option<(f64, f64)>>> = Arc::new(Mutex::new(None));

    let callback = move |event: Event| {
        match event.event_type {
            EventType::ButtonPress(button) => {
                let btn_id = button_id(&button);
                let now = Instant::now();

                let mut clicks = last_clicks.lock().unwrap();

                if let Some(last_time) = clicks.get(btn_id) {
                    let elapsed = now.duration_since(*last_time);

                    if elapsed < Duration::from_millis(DEBOUNCE_MS) {
                        // This is a bounce - always show, even in silent mode
                        println!(
                            "[BLOCKED] Bounce click: {:?} ({}ms after last)",
                            button,
                            elapsed.as_millis()
                        );
                        return;
                    }
                }

                // Valid click - update the timestamp
                if verbose {
                    println!("[OK] Valid click: {:?}", button);
                }
                clicks.insert(btn_id, now);
            }

            EventType::MouseMove { x, y } => {
                if !verbose {
                    return; // Skip all movement logging in silent mode
                }

                let mut pos = last_position.lock().unwrap();

                if let Some((last_x, last_y)) = *pos {
                    // Calculate distance moved
                    let dx = x - last_x;
                    let dy = y - last_y;
                    let distance = (dx * dx + dy * dy).sqrt();

                    if distance < MOVEMENT_THRESHOLD {
                        // Jitter - ignore this small movement
                        return;
                    }

                    // Significant movement - update reference and report
                    println!(
                        "[OK] Mouse moved: ({:.0}, {:.0}) - distance: {:.1}px",
                        x, y, distance
                    );
                    *pos = Some((x, y));
                } else {
                    // First movement ever - just store it
                    println!("[OK] Initial position: ({:.0}, {:.0})", x, y);
                    *pos = Some((x, y));
                }
            }

            _ => {} // Ignore other events
        }
    };

    // Start listening
    if let Err(e) = listen(callback) {
        eprintln!("Error: {:?}", e);
    }
}
