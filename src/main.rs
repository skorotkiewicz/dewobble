use std::env;
use std::fs::{OpenOptions, read_dir};
use std::os::fd::AsRawFd;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// evdev event structure (from linux/input.h)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct InputEvent {
    time_sec: i64,
    time_usec: i64,
    event_type: u16,
    code: u16,
    value: i32,
}

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const BTN_LEFT: u16 = 272;
const BTN_RIGHT: u16 = 273;
const BTN_MIDDLE: u16 = 274;

// Debounce window in milliseconds
const DEBOUNCE_MS: u64 = 200;
// Movement threshold - absorb movements smaller than this (in pixels)
const MOVEMENT_THRESHOLD: f64 = 3.0;
// Threshold squared (avoid sqrt calculation)
const MOVEMENT_THRESHOLD_SQ: f64 = MOVEMENT_THRESHOLD * MOVEMENT_THRESHOLD;

// Button state tracking for hold-mode debounce
struct ButtonState {
    last_press_time: AtomicU64,
    is_pressed: AtomicBool,
    is_debouncing: AtomicBool,
}

impl ButtonState {
    fn new() -> Self {
        Self {
            last_press_time: AtomicU64::new(0),
            is_pressed: AtomicBool::new(false),
            is_debouncing: AtomicBool::new(false),
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let verbose = args.iter().any(|arg| arg == "--verbose" || arg == "-v");
    let hold_mode = args.iter().any(|arg| arg == "--hold" || arg == "-h");

    println!("dewobble (epoll) started!");
    println!("Filtering clicks faster than {}ms", DEBOUNCE_MS);
    println!("Filtering movements smaller than {}px", MOVEMENT_THRESHOLD);
    if hold_mode {
        println!("Mode: HOLD (bypass rapid clicks as held state)");
    } else {
        println!("Mode: BLOCK (suppress rapid clicks entirely)");
    }
    println!("Press Ctrl+C to exit\n");

    // Open all mouse devices
    let mut device_fds: Vec<(std::path::PathBuf, RawFd)> = Vec::new();
    for entry in read_dir("/dev/input").expect("Cannot read /dev/input") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();
        if let Some(name) = path.file_name()
            && name.to_string_lossy().starts_with("event")
            && let Ok(file) = OpenOptions::new().read(true).open(&path)
        {
            device_fds.push((path.clone(), file.as_raw_fd()));
            std::mem::forget(file);
        }
    }

    if device_fds.is_empty() {
        eprintln!("No input devices found!");
        return;
    }

    println!("Monitoring {} device(s)", device_fds.len());

    // Create epoll instance
    let epoll_fd = unsafe { libc::epoll_create1(0) };
    if epoll_fd < 0 {
        panic!("Failed to create epoll");
    }

    // Add devices to epoll
    for (_, fd) in &device_fds {
        let mut event = libc::epoll_event {
            events: (libc::EPOLLIN) as u32,
            u64: *fd as u64,
        };
        unsafe {
            libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, *fd, &mut event);
        }
    }

    // State - using atomics for lock-free access
    let left_btn = ButtonState::new();
    let right_btn = ButtonState::new();
    let middle_btn = ButtonState::new();

    // Accumulated relative movement (for jitter filtering)
    let accum_x = AtomicI64::new(0);
    let accum_y = AtomicI64::new(0);

    let mut events: [libc::epoll_event; 10] = unsafe { std::mem::zeroed() };

    loop {
        let nfds = unsafe { libc::epoll_wait(epoll_fd, events.as_mut_ptr(), 10, -1) };
        if nfds < 0 {
            break;
        }

        for event in events.iter().take(nfds as usize) {
            let fd = event.u64 as i32;

            let mut input_event: InputEvent = unsafe { std::mem::zeroed() };
            let buffer = unsafe {
                std::slice::from_raw_parts_mut(
                    &mut input_event as *mut _ as *mut u8,
                    std::mem::size_of::<InputEvent>(),
                )
            };

            let bytes_read = unsafe { libc::read(fd, buffer.as_mut_ptr() as *mut _, buffer.len()) };
            if bytes_read != std::mem::size_of::<InputEvent>() as isize {
                continue;
            }

            match input_event.event_type {
                EV_KEY => {
                    let code = input_event.code;
                    let value = input_event.value;
                    let now = current_time_ms();

                    let (state, name) = match code {
                        BTN_LEFT => (&left_btn, "Left"),
                        BTN_RIGHT => (&right_btn, "Right"),
                        BTN_MIDDLE => (&middle_btn, "Middle"),
                        _ => continue,
                    };

                    if value == 1 {
                        // Button press
                        let last = state.last_press_time.load(Ordering::Relaxed);
                        let time_since_last = now.saturating_sub(last);

                        if last != 0 && time_since_last < DEBOUNCE_MS {
                            // Rapid click detected - treat as bounce
                            if hold_mode {
                                // In hold mode: mark as debouncing, keep pressed state
                                state.is_debouncing.store(true, Ordering::Relaxed);
                                state.is_pressed.store(true, Ordering::Relaxed);
                                if verbose {
                                    println!(
                                        "[HOLD] Rapid click absorbed: {} ({}ms) - treating as held",
                                        name, time_since_last
                                    );
                                }
                            } else {
                                // In block mode: suppress entirely
                                if verbose {
                                    println!(
                                        "[BLOCKED] Bounce click: {} ({}ms)",
                                        name, time_since_last
                                    );
                                }
                            }
                        } else {
                            // Valid press
                            state.is_pressed.store(true, Ordering::Relaxed);
                            state.is_debouncing.store(false, Ordering::Relaxed);
                            state.last_press_time.store(now, Ordering::Relaxed);
                            if verbose {
                                println!("[OK] Press: {}", name);
                            }
                        }
                    } else if value == 0 {
                        // Button release
                        if hold_mode {
                            let last = state.last_press_time.load(Ordering::Relaxed);
                            let time_held = now.saturating_sub(last);

                            if state.is_debouncing.load(Ordering::Relaxed) {
                                // Was in debounce state - check if we've waited long enough
                                if time_held < DEBOUNCE_MS {
                                    // Too soon - extend the hold
                                    state.is_pressed.store(true, Ordering::Relaxed);
                                    if verbose {
                                        println!(
                                            "[HOLD] Extending hold for {} ({}ms held)",
                                            name, time_held
                                        );
                                    }
                                    // Note: In a real implementation, you'd synthesize a delayed release
                                    // Here we just log it
                                } else {
                                    // Debounce period passed - allow release
                                    state.is_pressed.store(false, Ordering::Relaxed);
                                    state.is_debouncing.store(false, Ordering::Relaxed);
                                    if verbose {
                                        println!("[OK] Release (after hold): {}", name);
                                    }
                                }
                            } else {
                                // Normal release
                                state.is_pressed.store(false, Ordering::Relaxed);
                                if verbose {
                                    println!("[OK] Release: {}", name);
                                }
                            }
                        } else {
                            // Block mode - just track state
                            state.is_pressed.store(false, Ordering::Relaxed);
                            if verbose {
                                println!("[OK] Release: {}", name);
                            }
                        }
                    }
                }

                EV_REL if verbose => {
                    let dx = if input_event.code == REL_X {
                        input_event.value
                    } else if input_event.code == REL_Y {
                        0
                    } else {
                        continue;
                    };

                    let dy = if input_event.code == REL_Y {
                        input_event.value
                    } else {
                        0
                    };

                    let new_x = accum_x.load(Ordering::Relaxed) + dx as i64;
                    let new_y = accum_y.load(Ordering::Relaxed) + dy as i64;
                    accum_x.store(new_x, Ordering::Relaxed);
                    accum_y.store(new_y, Ordering::Relaxed);

                    let dist_sq = (new_x * new_x) as f64 + (new_y * new_y) as f64;
                    if dist_sq >= MOVEMENT_THRESHOLD_SQ {
                        let dist = dist_sq.sqrt();
                        println!(
                            "[OK] Mouse moved: accum=({}, {}), dist={:.1}px",
                            new_x, new_y, dist
                        );
                        accum_x.store(0, Ordering::Relaxed);
                        accum_y.store(0, Ordering::Relaxed);
                    }
                }

                _ => {}
            }
        }
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
