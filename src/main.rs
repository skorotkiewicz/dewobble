use std::env;
use std::fs::{File, OpenOptions, read_dir};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

impl InputEvent {
    fn new(event_type: u16, code: u16, value: i32) -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        Self {
            time_sec: now.as_secs() as i64,
            time_usec: now.as_micros() as i64 % 1_000_000,
            event_type,
            code,
            value,
        }
    }
}

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_SYN: u16 = 0;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const BTN_LEFT: u16 = 272;
const BTN_RIGHT: u16 = 273;
const BTN_MIDDLE: u16 = 274;
const SYN_REPORT: u16 = 0;

// uinput constants
const UI_SET_EVBIT: u64 = 0x40045564;
const UI_SET_KEYBIT: u64 = 0x40045565;
const UI_SET_RELBIT: u64 = 0x40045566;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;

#[repr(C)]
struct UinputSetup {
    name: [u8; 80],
    id: InputId,
}

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

// Debounce window in milliseconds
const DEBOUNCE_MS: u64 = 200;
// Movement threshold - absorb movements smaller than this (in pixels)
const MOVEMENT_THRESHOLD: f64 = 3.0;
const MOVEMENT_THRESHOLD_SQ: f64 = MOVEMENT_THRESHOLD * MOVEMENT_THRESHOLD;

// Button state tracking for hold-mode debounce
#[derive(Clone)]
struct ButtonState {
    last_press_time: Arc<AtomicU64>,
    is_pressed: Arc<AtomicBool>,
    is_debouncing: Arc<AtomicBool>,
    pending_release: Arc<AtomicBool>,
}

impl ButtonState {
    fn new() -> Self {
        Self {
            last_press_time: Arc::new(AtomicU64::new(0)),
            is_pressed: Arc::new(AtomicBool::new(false)),
            is_debouncing: Arc::new(AtomicBool::new(false)),
            pending_release: Arc::new(AtomicBool::new(false)),
        }
    }
}

// Create virtual input device via uinput
fn create_virtual_device() -> Option<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/uinput")
        .ok()?;

    let fd = file.as_raw_fd();

    unsafe {
        // Enable event types
        libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_uint);
        libc::ioctl(fd, UI_SET_EVBIT, EV_REL as libc::c_uint);
        libc::ioctl(fd, UI_SET_EVBIT, EV_SYN as libc::c_uint);

        // Enable buttons
        libc::ioctl(fd, UI_SET_KEYBIT, BTN_LEFT as i32);
        libc::ioctl(fd, UI_SET_KEYBIT, BTN_RIGHT as i32);
        libc::ioctl(fd, UI_SET_KEYBIT, BTN_MIDDLE as i32);

        // Enable relative axes
        libc::ioctl(fd, UI_SET_RELBIT, REL_X as i32);
        libc::ioctl(fd, UI_SET_RELBIT, REL_Y as i32);

        // Setup device
        let mut setup: UinputSetup = std::mem::zeroed();
        let name = b"dewobble-virtual-mouse\0";
        setup.name[..name.len()].copy_from_slice(name);
        setup.id = InputId {
            bustype: 0x03, // BUS_USB
            vendor: 0x1234,
            product: 0x5678,
            version: 1,
        };

        libc::write(
            fd,
            &setup as *const _ as *const _,
            std::mem::size_of::<UinputSetup>(),
        );
        libc::ioctl(fd, UI_DEV_CREATE, 0);
    }

    Some(file)
}

// Emit event to virtual device
fn emit_event(device: &File, event_type: u16, code: u16, value: i32) {
    let event = InputEvent::new(event_type, code, value);
    unsafe {
        libc::write(
            device.as_raw_fd(),
            &event as *const _ as *const _,
            std::mem::size_of::<InputEvent>(),
        );
    }
}

// Sync events
fn sync_events(device: &File) {
    emit_event(device, EV_SYN, SYN_REPORT, 0);
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let verbose = args.iter().any(|arg| arg == "--verbose" || arg == "-v");
    let hold_mode = args.iter().any(|arg| arg == "--hold" || arg == "-h");

    println!("dewobble (epoll + uinput) started!");
    println!("Filtering clicks faster than {}ms", DEBOUNCE_MS);
    println!("Filtering movements smaller than {}px", MOVEMENT_THRESHOLD);

    // Create virtual output device
    let virtual_dev = create_virtual_device();
    if virtual_dev.is_none() {
        eprintln!("Warning: Could not create virtual device. Running in monitoring mode only.");
        eprintln!("Try: sudo modprobe uinput");
    } else {
        println!(
            "Virtual mouse created: /dev/input/eventX (use this instead of your physical mouse)"
        );
    }
    let vdev = virtual_dev.as_ref();

    if hold_mode {
        println!("Mode: HOLD (rapid clicks are converted to held state)");
    } else {
        println!("Mode: BLOCK (rapid clicks are suppressed)");
    }
    if verbose {
        println!("Verbose mode: ON");
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

    // Button states
    let left_btn = ButtonState::new();
    let right_btn = ButtonState::new();
    let middle_btn = ButtonState::new();

    // Movement accumulation
    let accum_x = AtomicI64::new(0);
    let accum_y = AtomicI64::new(0);

    let mut events: [libc::epoll_event; 10] = unsafe { std::mem::zeroed() };

    // Spawn thread for delayed releases in hold mode
    if hold_mode && vdev.is_some() {
        let vdev_fd = virtual_dev.as_ref().unwrap().try_clone().unwrap();
        let left_clone = left_btn.clone();
        let right_clone = right_btn.clone();
        let middle_clone = middle_btn.clone();
        let verbose_clone = verbose;

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(10));
                let now = current_time_ms();

                for (state, code, name) in [
                    (&left_clone, BTN_LEFT, "Left"),
                    (&right_clone, BTN_RIGHT, "Right"),
                    (&middle_clone, BTN_MIDDLE, "Middle"),
                ] {
                    if state.pending_release.load(Ordering::Relaxed) {
                        let last = state.last_press_time.load(Ordering::Relaxed);
                        if now - last >= DEBOUNCE_MS {
                            // Time to release
                            state.is_pressed.store(false, Ordering::Relaxed);
                            state.is_debouncing.store(false, Ordering::Relaxed);
                            state.pending_release.store(false, Ordering::Relaxed);

                            // Emit release event
                            emit_event(&vdev_fd, EV_KEY, code, 0);
                            sync_events(&vdev_fd);

                            if verbose_clone {
                                println!("[HOLD] Auto-released {} after debounce period", name);
                            }
                        }
                    }
                }
            }
        });
    }

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
                        _ => {
                            // Pass through unhandled buttons
                            if let Some(dev) = vdev {
                                emit_event(dev, EV_KEY, code, value);
                                sync_events(dev);
                            }
                            continue;
                        }
                    };

                    if value == 1 {
                        // Button press
                        let last = state.last_press_time.load(Ordering::Relaxed);
                        let time_since_last = now.saturating_sub(last);

                        if last != 0 && time_since_last < DEBOUNCE_MS {
                            // Rapid click
                            if hold_mode {
                                state.is_debouncing.store(true, Ordering::Relaxed);
                                state.is_pressed.store(true, Ordering::Relaxed);
                                state.pending_release.store(false, Ordering::Relaxed);

                                // Emit press to virtual device (maintain held state)
                                if let Some(dev) = vdev {
                                    emit_event(dev, EV_KEY, code, 1);
                                    sync_events(dev);
                                }

                                if verbose {
                                    println!(
                                        "[HOLD] Absorbed bounce, maintaining hold: {} ({}ms)",
                                        name, time_since_last
                                    );
                                }
                            } else {
                                // Block mode - suppress entirely
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
                            state.pending_release.store(false, Ordering::Relaxed);
                            state.last_press_time.store(now, Ordering::Relaxed);

                            // Emit to virtual device
                            if let Some(dev) = vdev {
                                emit_event(dev, EV_KEY, code, 1);
                                sync_events(dev);
                            }

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
                                if time_held < DEBOUNCE_MS {
                                    // Too soon - mark for delayed release
                                    state.pending_release.store(true, Ordering::Relaxed);
                                    if verbose {
                                        println!(
                                            "[HOLD] Delaying release for {} ({}ms held)",
                                            name, time_held
                                        );
                                    }
                                    // Don't emit release yet - thread will handle it
                                } else {
                                    // Debounce passed
                                    state.is_pressed.store(false, Ordering::Relaxed);
                                    state.is_debouncing.store(false, Ordering::Relaxed);
                                    state.pending_release.store(false, Ordering::Relaxed);

                                    if let Some(dev) = vdev {
                                        emit_event(dev, EV_KEY, code, 0);
                                        sync_events(dev);
                                    }

                                    if verbose {
                                        println!("[OK] Release (after hold): {}", name);
                                    }
                                }
                            } else {
                                // Normal release
                                state.is_pressed.store(false, Ordering::Relaxed);
                                state.pending_release.store(false, Ordering::Relaxed);

                                if let Some(dev) = vdev {
                                    emit_event(dev, EV_KEY, code, 0);
                                    sync_events(dev);
                                }

                                if verbose {
                                    println!("[OK] Release: {}", name);
                                }
                            }
                        } else {
                            // Block mode - pass through
                            state.is_pressed.store(false, Ordering::Relaxed);

                            if let Some(dev) = vdev {
                                emit_event(dev, EV_KEY, code, 0);
                                sync_events(dev);
                            }

                            if verbose {
                                println!("[OK] Release: {}", name);
                            }
                        }
                    }
                }

                EV_REL => {
                    let rel_code = input_event.code;
                    let rel_value = input_event.value;

                    if rel_code == REL_X || rel_code == REL_Y {
                        // Accumulate for jitter filtering
                        let is_x = rel_code == REL_X;
                        let val = rel_value as i64;

                        if is_x {
                            accum_x.fetch_add(val, Ordering::Relaxed);
                        } else {
                            accum_y.fetch_add(val, Ordering::Relaxed);
                        }

                        let ax = accum_x.load(Ordering::Relaxed);
                        let ay = accum_y.load(Ordering::Relaxed);
                        let dist_sq = (ax * ax) as f64 + (ay * ay) as f64;

                        if dist_sq >= MOVEMENT_THRESHOLD_SQ {
                            // Significant movement - emit to virtual device
                            if let Some(dev) = vdev {
                                if ax != 0 {
                                    emit_event(dev, EV_REL, REL_X, ax as i32);
                                }
                                if ay != 0 {
                                    emit_event(dev, EV_REL, REL_Y, ay as i32);
                                }
                                sync_events(dev);
                            }

                            if verbose {
                                let dist = dist_sq.sqrt();
                                println!("[OK] Mouse moved: ({}, {}), dist={:.1}px", ax, ay, dist);
                            }

                            accum_x.store(0, Ordering::Relaxed);
                            accum_y.store(0, Ordering::Relaxed);
                        }
                    } else {
                        // Other relative events - pass through
                        if let Some(dev) = vdev {
                            emit_event(dev, EV_REL, rel_code, rel_value);
                        }
                    }
                }

                EV_SYN => {
                    // Sync events - pass through if we have a virtual device
                    if let Some(dev) = vdev {
                        emit_event(dev, EV_SYN, input_event.code, input_event.value);
                    }
                }

                _ => {
                    // Pass through unhandled event types
                    if let Some(dev) = vdev {
                        emit_event(
                            dev,
                            input_event.event_type,
                            input_event.code,
                            input_event.value,
                        );
                    }
                }
            }
        }
    }

    // Cleanup
    if let Some(dev) = virtual_dev {
        unsafe { libc::ioctl(dev.as_raw_fd(), UI_DEV_DESTROY, 0) };
    }
}
