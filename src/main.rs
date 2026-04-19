use rdev::{Button, Event, EventType, listen};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Minimum time between clicks to be considered separate (not bounce)
// Physically faulty switches usually bounce within 20-80ms
const DEBOUNCE_MS: u64 = 100;

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
    println!("Mouse Debouncer started!");
    println!("Filtering out clicks faster than {}ms", DEBOUNCE_MS);
    println!("Press Ctrl+C to exit\n");

    // Track last click time for each button
    let last_clicks: Arc<Mutex<HashMap<&'static str, Instant>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let callback = move |event: Event| {
        // Only process button press events
        if let EventType::ButtonPress(button) = event.event_type {
            let btn_id = button_id(&button);
            let now = Instant::now();

            let mut clicks = last_clicks.lock().unwrap();

            if let Some(last_time) = clicks.get(btn_id) {
                let elapsed = now.duration_since(*last_time);

                if elapsed < Duration::from_millis(DEBOUNCE_MS) {
                    // This is a bounce - ignore it
                    println!(
                        "Blocked bounce: {:?} ({}ms after last click)",
                        button,
                        elapsed.as_millis()
                    );
                    return;
                }
            }

            // Valid click - update the timestamp
            println!("Valid click: {:?}", button);
            clicks.insert(btn_id, now);
        }
    };

    // Start listening
    if let Err(e) = listen(callback) {
        eprintln!("Error: {:?}", e);
    }
}
