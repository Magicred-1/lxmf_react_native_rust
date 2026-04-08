use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

const LOG_BUFFER_CAPACITY: usize = 400;

#[derive(Debug, Clone)]
pub struct QueuedLog {
    pub level: u32,
    pub message: String,
}

struct BridgeLogger;
static LOGGER: BridgeLogger = BridgeLogger;

static LOG_QUEUE: OnceLock<Mutex<VecDeque<QueuedLog>>> = OnceLock::new();
static LOGGER_INIT: OnceLock<()> = OnceLock::new();

fn queue() -> &'static Mutex<VecDeque<QueuedLog>> {
    LOG_QUEUE.get_or_init(|| Mutex::new(VecDeque::with_capacity(LOG_BUFFER_CAPACITY)))
}

pub fn init_logger(level: LevelFilter) {
    if LOGGER_INIT.get().is_none() {
        if log::set_logger(&LOGGER).is_ok() {
            let _ = LOGGER_INIT.set(());
        }
    }
    log::set_max_level(level);
}

pub fn set_max_level_from_u32(level: u32) {
    let mapped = match level {
        0 => LevelFilter::Error,
        1 => LevelFilter::Warn,
        2 => LevelFilter::Info,
        3 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };
    log::set_max_level(mapped);
}

pub fn drain_logs() -> Vec<QueuedLog> {
    let mut out = Vec::new();
    if let Ok(mut q) = queue().lock() {
        while let Some(entry) = q.pop_front() {
            out.push(entry);
        }
    }
    out
}

impl Log for BridgeLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let rendered = if record.target().is_empty() {
            format!("{}", record.args())
        } else {
            format!("{}: {}", record.target(), record.args())
        };

        android_write(record.level(), &rendered);

        if let Ok(mut q) = queue().lock() {
            if q.len() >= LOG_BUFFER_CAPACITY {
                q.pop_front();
            }
            q.push_back(QueuedLog {
                level: level_to_u32(record.level()),
                message: rendered,
            });
        }
    }

    fn flush(&self) {}
}

fn level_to_u32(level: Level) -> u32 {
    match level {
        Level::Error => 0,
        Level::Warn => 1,
        Level::Info => 2,
        Level::Debug => 3,
        Level::Trace => 4,
    }
}

#[cfg(target_os = "android")]
fn android_write(level: Level, message: &str) {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};

    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    let tag = CString::new("LxmfRust").expect("static tag is valid CString");
    let clean = message.replace('\0', "\\u{0}");
    if let Ok(msg) = CString::new(clean) {
        let prio = match level {
            Level::Error => 6,
            Level::Warn => 5,
            Level::Info => 4,
            Level::Debug => 3,
            Level::Trace => 2,
        };
        unsafe {
            let _ = __android_log_write(prio, tag.as_ptr(), msg.as_ptr());
        }
    }
}

#[cfg(not(target_os = "android"))]
fn android_write(level: Level, message: &str) {
    eprintln!("[LxmfRust][{}] {}", level, message);
}