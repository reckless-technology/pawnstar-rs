//! Debugging counts dictionary. (Port of `debug_hashtable.h`.)
//!
//! The C++ keeps a global `unordered_map<string_view, int64_t>` where each INCREMENT call site
//! caches a stable slot pointer in a function-local static. The port keeps the same shape:
//! a global registry of leaked `AtomicI64` slots (stable for the program's lifetime), with each
//! `increment!` call site caching its slot in a `OnceLock`. C++ increments are plain (racy)
//! int64 under Lazy SMP; relaxed atomics here are exactly equivalent single-threaded — the only
//! mode the parity gate compares — and merely lose no updates multi-threaded.

#[cfg(feature = "debugx")]
mod imp {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Mutex;

    /// Global dictionary of named diagnostic counters.
    static DEBUG_DICTIONARY: Mutex<BTreeMap<&'static str, &'static AtomicI64>> =
        Mutex::new(BTreeMap::new());

    /// Find-or-create the counter slot for a name and return a stable reference to it.
    /// Slots are leaked (never freed) so the references cached at each `increment!` call site
    /// remain valid for the program's lifetime. Creation is serialised so concurrent first-time
    /// creations from worker threads don't race on the map.
    pub fn debug_slot(name: &'static str) -> &'static AtomicI64 {
        let mut map = DEBUG_DICTIONARY.lock().unwrap();
        map.entry(name)
            .or_insert_with(|| Box::leak(Box::new(AtomicI64::new(0))))
    }

    /// Reset all diagnostic counters to zero. Entries are kept (not erased) so the slot pointers
    /// cached at each `increment!` call site remain valid.
    pub fn debug_x_clear() {
        let map = DEBUG_DICTIONARY.lock().unwrap();
        for slot in map.values() {
            slot.store(0, Ordering::Relaxed);
        }
    }

    /// Print the non-zero debug counters in alphabetic order (byte-identical to the C++ format).
    pub fn debug_x_write() {
        let map = DEBUG_DICTIONARY.lock().unwrap();
        print!("********************* DEBUGX *********************\n");
        for (title, slot) in map.iter() {
            let count = slot.load(Ordering::Relaxed);
            if count != 0 {
                print!("{:<40}{:10}\n", title, count);
            }
        }
        print!("**************************************************\n");
    }
}

#[cfg(feature = "debugx")]
pub use imp::{debug_slot, debug_x_clear, debug_x_write};

#[cfg(not(feature = "debugx"))]
mod imp_noop {
    /// No-op when diagnostics are disabled.
    pub fn debug_x_clear() {}
    /// No-op when diagnostics are disabled.
    pub fn debug_x_write() {}
}

#[cfg(not(feature = "debugx"))]
pub use imp_noop::{debug_x_clear, debug_x_write};

/// Increment the named counter (slot reference cached per call site).
#[cfg(feature = "debugx")]
#[macro_export]
macro_rules! increment {
    ($name:expr) => {{
        static SLOT: std::sync::OnceLock<&'static std::sync::atomic::AtomicI64> =
            std::sync::OnceLock::new();
        SLOT.get_or_init(|| $crate::debug_hashtable::debug_slot($name))
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }};
}

/// No-op when diagnostics are disabled.
#[cfg(not(feature = "debugx"))]
#[macro_export]
macro_rules! increment {
    ($name:expr) => {{}};
}

/// Set the named counter to a value (slot reference cached per call site).
#[cfg(feature = "debugx")]
#[macro_export]
macro_rules! assign {
    ($name:expr, $val:expr) => {{
        static SLOT: std::sync::OnceLock<&'static std::sync::atomic::AtomicI64> =
            std::sync::OnceLock::new();
        SLOT.get_or_init(|| $crate::debug_hashtable::debug_slot($name))
            .store($val, std::sync::atomic::Ordering::Relaxed);
    }};
}

/// No-op when diagnostics are disabled.
#[cfg(not(feature = "debugx"))]
#[macro_export]
macro_rules! assign {
    ($name:expr, $val:expr) => {{
        let _ = $val;
    }};
}

/// Increment the named counter when the condition is true (slot reference cached per call site).
#[cfg(feature = "debugx")]
#[macro_export]
macro_rules! increment_if {
    ($b:expr, $name:expr) => {{
        static SLOT: std::sync::OnceLock<&'static std::sync::atomic::AtomicI64> =
            std::sync::OnceLock::new();
        if $b {
            SLOT.get_or_init(|| $crate::debug_hashtable::debug_slot($name))
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else {
            // Match the C++ macro: the slot is created (and thus listed by `dbg`) even when the
            // condition is false, because DebugSlot runs unconditionally in the C++ do-while.
            SLOT.get_or_init(|| $crate::debug_hashtable::debug_slot($name));
        }
    }};
}

/// No-op when diagnostics are disabled.
#[cfg(not(feature = "debugx"))]
#[macro_export]
macro_rules! increment_if {
    ($b:expr, $name:expr) => {{
        let _ = $b;
    }};
}
