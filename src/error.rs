//! Error reporting that mirrors the exact message text libSiON-cpp emitted
//! (itself Godot 4.x `err_print_error` parity). The mml-compilation golden
//! outputs consist of the `ERROR: ` first lines, so the formatted shape must
//! stay in sync:
//!
//! ```text
//! ERROR: <message>
//! <error>                       (only when both message and error exist)
//!   at: <function> (<file>:<line>)
//! ```
//!
//! Golden files only ever capture the `ERROR: ` line. When porting a C++
//! `ERR_*` macro, the first line must be reproduced **verbatim** — for the
//! no-custom-message variants that means the stringified C++ condition text.

use std::io::Write;
use std::sync::Mutex;
use std::sync::OnceLock;

type ErrorOutput = Box<dyn Fn(&str) + Send + Sync>;

static ERROR_SINK: OnceLock<Mutex<Option<ErrorOutput>>> = OnceLock::new();

fn sink_mutex() -> &'static Mutex<Option<ErrorOutput>> {
    ERROR_SINK.get_or_init(|| Mutex::new(None))
}

/// Replace the error sink (tests capture output here). `None` restores stderr.
pub fn set_error_output(f: Option<ErrorOutput>) {
    *sink_mutex().lock().unwrap() = f;
}

/// Emit a fully formatted Godot-parity error block. `body` is everything that
/// goes on the `ERROR:` line (and optional following lines); the `at:` trailer
/// uses the Rust location, which goldens do not compare.
pub fn err_print_body(body: &str, warning: bool) {
    let level = if warning { "WARNING" } else { "ERROR" };
    let out = format!("{level}: {body}\n  at: {}\n", file!());
    let guard = sink_mutex().lock().unwrap();
    match guard.as_ref() {
        Some(f) => f(&out),
        None => {
            let mut err = std::io::stderr();
            let _ = err.write_all(out.as_bytes());
            let _ = err.flush();
        }
    }
}

/// Emit one raw formatted line, `at:`-trailer included.
pub fn err_print_raw(out: &str) {
    let guard = sink_mutex().lock().unwrap();
    match guard.as_ref() {
        Some(f) => f(out),
        None => {
            let mut err = std::io::stderr();
            let _ = err.write_all(out.as_bytes());
            let _ = err.flush();
        }
    }
}

/// C++ `ERR_PRINT(msg)` / bare error line. First line = message.
#[macro_export]
macro_rules! err_print {
    ($($arg:tt)*) => { $crate::error::err_print_body(&format!($($arg)*), false) };
}

/// C++ `WARN_PRINT(msg)`.
#[macro_export]
macro_rules! warn_print {
    ($($arg:tt)*) => { $crate::error::err_print_body(&format!($($arg)*), true) };
}

/// C++ `ERR_FAIL_COND(cond)` — `cond_text` must be the *verbatim C++ source*
/// of the condition (it lands in the `ERROR:` line).
#[macro_export]
macro_rules! err_fail_cond {
    ($cond:expr, $cond_text:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!("Condition \"{}\" is true.", $cond_text),
                false,
            );
            return;
        }
    };
}

/// C++ `ERR_FAIL_COND_MSG(cond, msg)` — per `sion_errors.h::err_print`, the
/// custom message is the FIRST line (golden-critical); the stringified
/// condition follows it.
#[macro_export]
macro_rules! err_fail_cond_msg {
    ($cond:expr, $cond_text:expr, $msg:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!("{}\nCondition \"{}\" is true.", $msg, $cond_text),
                false,
            );
            return;
        }
    };
}

/// C++ `ERR_CONTINUE_MSG(cond, msg)` — custom message first (golden-critical
/// first line), `Condition "..." is true. Continued.` second.
#[macro_export]
macro_rules! err_continue_msg {
    ($cond:expr, $cond_text:expr, $msg:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!("{}\nCondition \"{}\" is true. Continued.", $msg, $cond_text),
                false,
            );
            continue;
        }
    };
}

/// C++ `ERR_CONTINUE(cond)`.
#[macro_export]
macro_rules! err_continue {
    ($cond:expr, $cond_text:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!("Condition \"{}\" is true.", $cond_text),
                false,
            );
            continue;
        }
    };
}

/// C++ `ERR_BREAK(cond)` / `ERR_BREAK_MSG`.
#[macro_export]
macro_rules! err_break {
    ($cond:expr, $cond_text:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!("Condition \"{}\" is true.", $cond_text),
                false,
            );
            break;
        }
    };
}

/// C++ `ERR_FAIL_COND_V(cond, val)`.
#[macro_export]
macro_rules! err_fail_cond_v {
    ($cond:expr, $cond_text:expr, $val:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!("Condition \"{}\" is true. Returning: {}", $cond_text, $val),
                false,
            );
            return $val;
        }
    };
}

/// C++ `ERR_FAIL_COND_V_MSG(cond, val, msg)` — message first (golden line).
#[macro_export]
macro_rules! err_fail_cond_v_msg {
    ($cond:expr, $cond_text:expr, $val:expr, $msg:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!(
                    "{}\nCondition \"{}\" is true. Returning: {}",
                    $msg, $cond_text, $val
                ),
                false,
            );
            return $val;
        }
    };
}

/// C++ `ERR_FAIL_INDEX(i, size)` — `i_text`/`size_text` are verbatim C++.
#[macro_export]
macro_rules! err_fail_index {
    ($idx:expr, $i_text:expr, $size:expr, $size_text:expr) => {
        if $idx < 0 || $idx >= $size {
            $crate::error::err_print_body(
                &format!(
                    "Index {} = {} is out of bounds ({} = {}).",
                    $i_text, $idx, $size_text, $size
                ),
                false,
            );
            return;
        }
    };
}

/// C++ `ERR_FAIL_INDEX_V(i, size, val)` — compat header uses the SAME format
/// string as ERR_FAIL_INDEX (no "Returning:" suffix).
#[macro_export]
macro_rules! err_fail_index_v {
    ($idx:expr, $i_text:expr, $size:expr, $size_text:expr, $val:expr) => {
        if $idx < 0 || $idx >= $size {
            $crate::error::err_print_body(
                &format!(
                    "Index {} = {} is out of bounds ({} = {}).",
                    $i_text, $idx, $size_text, $size
                ),
                false,
            );
            return $val;
        }
    };
}

/// C++ `ERR_FAIL_NULL(p)` (Option-taking).
#[macro_export]
macro_rules! err_fail_null {
    ($opt:expr, $p_text:expr) => {
        if $opt.is_none() {
            $crate::error::err_print_body(
                &format!("Parameter \"{}\" is null.", $p_text),
                false,
            );
            return;
        }
    };
}

/// C++ `ERR_FAIL_NULL_V(p, val)`.
#[macro_export]
macro_rules! err_fail_null_v {
    ($opt:expr, $p_text:expr, $val:expr) => {
        if $opt.is_none() {
            $crate::error::err_print_body(
                &format!("Parameter \"{}\" is null. Returning: {}", $p_text, $val),
                false,
            );
            return $val;
        }
    };
}

/// C++ `ERR_FAIL()` / `ERR_FAIL_MSG(msg)`.
#[macro_export]
macro_rules! err_fail {
    () => {{
        $crate::error::err_print_body("Method/function failed.", false);
        return;
    }};
    ($msg:expr) => {{
        $crate::error::err_print_body(&format!("{}\nMethod/function failed.", $msg), false);
        return;
    }};
}
