use std::env;
use std::fs::{OpenOptions, read_dir};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
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
const BTN_LEFT: u16 = 272;
const BTN_RIGHT: u16 = 273;
const BTN_MIDDLE: u16 = 274;

// Debounce window in milliseconds
const DEBOUNCE_MS: u64 = 100;

fn main() {
    let verbose = env::args().any(|arg| arg == "--verbose" || arg == "-v");

    println!("dewobble (epoll) started!");
    println!("Filtering clicks faster than {}ms", DEBOUNCE_MS);
    println!("Press Ctrl+C to exit\n");

    // Open all mouse devices
    let mut device_fds: Vec<(std::path::PathBuf, std::os::fd::RawFd)> = Vec::new();
    for entry in read_dir("/dev/input").expect("Cannot read /dev/input") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();
        if let Some(name) = path.file_name()
            && name.to_string_lossy().starts_with("event")
            && let Ok(file) = OpenOptions::new().read(true).open(&path)
        {
            device_fds.push((path.clone(), file.as_raw_fd()));
            // Keep file open by leaking it (simpler than managing ownership)
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
    let last_click_left = AtomicU64::new(0);
    let last_click_right = AtomicU64::new(0);
    let last_click_middle = AtomicU64::new(0);

    let mut events: [libc::epoll_event; 10] = unsafe { std::mem::zeroed() };

    loop {
        let nfds = unsafe { libc::epoll_wait(epoll_fd, events.as_mut_ptr(), 10, -1) };
        if nfds < 0 {
            break; // Interrupted
        }

        for event in events.iter().take(nfds as usize) {
            let fd = event.u64 as i32;

            // Read the event directly from the fd
            let mut input_event: InputEvent = unsafe { std::mem::zeroed() };
            let buffer = unsafe {
                std::slice::from_raw_parts_mut(
                    &mut input_event as *mut _ as *mut u8,
                    std::mem::size_of::<InputEvent>(),
                )
            };

            let bytes_read = unsafe { libc::read(fd, buffer.as_mut_ptr() as *mut _, buffer.len()) };
            if bytes_read == std::mem::size_of::<InputEvent>() as isize
                && input_event.event_type == EV_KEY
                && input_event.value == 1
            {
                // Button press
                let now = current_time_ms();
                let code = input_event.code;

                let (last_time_ref, name) = match code {
                    BTN_LEFT => (&last_click_left, "Left"),
                    BTN_RIGHT => (&last_click_right, "Right"),
                    BTN_MIDDLE => (&last_click_middle, "Middle"),
                    _ => continue,
                };

                let last = last_time_ref.load(Ordering::Relaxed);
                if last != 0 && now - last < DEBOUNCE_MS {
                    println!("[BLOCKED] Bounce click: {} ({}ms)", name, now - last);
                } else {
                    if verbose {
                        println!("[OK] Valid click: {}", name);
                    }
                    last_time_ref.store(now, Ordering::Relaxed);
                }
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
