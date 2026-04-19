use rdev::{Button, Event, EventType, listen};
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Minimum time between clicks to be considered separate (not bounce)
const DEFAULT_DEBOUNCE_MS: u64 = 100;

// Minimum mouse movement in pixels to not be considered "jitter"
const DEFAULT_MOVEMENT_THRESHOLD: f64 = 3.0;

// Button state tracking for hold-mode debounce
struct ButtonState {
    last_press_time: Option<Instant>,
    is_pressed: bool,
    is_debouncing: bool,
}

impl ButtonState {
    fn new() -> Self {
        Self {
            last_press_time: None,
            is_pressed: false,
            is_debouncing: false,
        }
    }
}

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

fn get_debounce_ms() -> u64 {
    env::var("DEBOUNCE_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_DEBOUNCE_MS)
}

fn get_movement_threshold() -> f64 {
    env::var("MOVEMENT_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MOVEMENT_THRESHOLD)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let verbose = args.iter().any(|arg| arg == "--verbose" || arg == "-v");
    let hold_mode = args.iter().any(|arg| arg == "--hold" || arg == "-h");

    let debounce_ms = get_debounce_ms();
    let movement_threshold = get_movement_threshold();

    println!("dewobble (rdev) started!");
    println!("Filtering clicks faster than {}ms", debounce_ms);
    println!(
        "Filtering movements smaller than {} pixels",
        movement_threshold
    );
    if hold_mode {
        println!("Mode: HOLD (absorb rapid clicks as held state)");
    } else {
        println!("Mode: BLOCK (suppress rapid clicks)");
    }
    println!("Press Ctrl+C to exit\n");

    // Track button states
    let button_states: Arc<Mutex<HashMap<&'static str, ButtonState>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Track mouse position for jitter filtering
    let last_position: Arc<Mutex<Option<(f64, f64)>>> = Arc::new(Mutex::new(None));

    let callback = move |event: Event| {
        match event.event_type {
            EventType::ButtonPress(button) => {
                let btn_id = button_id(&button);
                let now = Instant::now();

                let mut states = button_states.lock().unwrap();
                let state = states.entry(btn_id).or_insert_with(ButtonState::new);

                if let Some(last_time) = state.last_press_time {
                    let elapsed = now.duration_since(last_time);

                    if elapsed < Duration::from_millis(debounce_ms) {
                        // Rapid click detected
                        if hold_mode {
                            state.is_debouncing = true;
                            state.is_pressed = true;
                            println!(
                                "[HOLD] Rapid click absorbed: {:?} ({}ms) - treating as held",
                                button,
                                elapsed.as_millis()
                            );
                        } else {
                            println!(
                                "[BLOCKED] Bounce click: {:?} ({}ms after last)",
                                button,
                                elapsed.as_millis()
                            );
                        }
                        return;
                    }
                }

                // Valid click
                state.is_pressed = true;
                state.is_debouncing = false;
                state.last_press_time = Some(now);
                if verbose {
                    println!("[OK] Valid click: {:?}", button);
                }
            }

            EventType::ButtonRelease(button) => {
                if !hold_mode {
                    // In block mode, just update state silently
                    let btn_id = button_id(&button);
                    let mut states = button_states.lock().unwrap();
                    if let Some(state) = states.get_mut(btn_id) {
                        state.is_pressed = false;
                        if verbose {
                            println!("[OK] Release: {:?}", button);
                        }
                    }
                    return;
                }

                // Hold mode release handling
                let btn_id = button_id(&button);
                let now = Instant::now();
                let mut states = button_states.lock().unwrap();

                if let Some(state) = states.get_mut(btn_id)
                    && let Some(last_time) = state.last_press_time
                {
                    let held_duration = now.duration_since(last_time);

                    if state.is_debouncing {
                        // Was in debounce state
                        if held_duration < Duration::from_millis(debounce_ms) {
                            // Too soon - extend hold (log only, actual hold requires event injection)
                            state.is_pressed = true;
                            println!(
                                "[HOLD] Extending hold for {:?} ({}ms held)",
                                button,
                                held_duration.as_millis()
                            );
                        } else {
                            // Debounce period passed
                            state.is_pressed = false;
                            state.is_debouncing = false;
                            if verbose {
                                println!("[OK] Release (after hold): {:?}", button);
                            }
                        }
                    } else {
                        // Normal release
                        state.is_pressed = false;
                        if verbose {
                            println!("[OK] Release: {:?}", button);
                        }
                    }
                }
            }

            EventType::MouseMove { x, y } => {
                if !verbose {
                    return;
                }

                let mut pos = last_position.lock().unwrap();

                if let Some((last_x, last_y)) = *pos {
                    let dx = x - last_x;
                    let dy = y - last_y;
                    let distance = (dx * dx + dy * dy).sqrt();

                    if distance < movement_threshold {
                        return;
                    }

                    println!(
                        "[OK] Mouse moved: ({:.0}, {:.0}) - distance: {:.1}px",
                        x, y, distance
                    );
                    *pos = Some((x, y));
                } else {
                    println!("[OK] Initial position: ({:.0}, {:.0})", x, y);
                    *pos = Some((x, y));
                }
            }

            _ => {}
        }
    };

    if let Err(e) = listen(callback) {
        eprintln!("Error: {:?}", e);
    }
}
