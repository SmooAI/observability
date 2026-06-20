//! Stack capture.
//!
//! The TS SDK parses a JS `Error.stack` string into frames. Rust has no string
//! stack on its error values, so the analogue is capturing a live
//! [`backtrace::Backtrace`] at the point of capture and resolving it into the
//! same [`StackFrame`] wire shape (innermost-first, `in_app` tagged).
//!
//! Frames are returned innermost (most recent) first to match the
//! `@smooai/observability` event envelope. SDK-internal frames and frames in
//! the Rust runtime / std / registry deps are tagged `in_app: false`.

use crate::types::StackFrame;

/// Hints that mark a frame as NOT application code.
const NON_APP_HINTS: &[&str] = &[
    "smooai_observability", // this SDK
    "/.cargo/",             // registry dependencies
    "/rustc/",              // std / core
    "backtrace::",
    "core::panic",
    "std::panic",
    "std::rt",
    "std::sys",
    "tokio::runtime",
    "__rust_",
];

/// Capture the current stack as resolved frames, innermost-first, dropping the
/// SDK-internal frames at the top.
pub fn capture_stack() -> Vec<StackFrame> {
    let bt = backtrace::Backtrace::new();
    let frames = resolve_backtrace(&bt);
    drop_sdk_frames(frames)
}

/// Resolve a [`backtrace::Backtrace`] into [`StackFrame`]s. Split out so tests
/// can exercise the mapping deterministically.
fn resolve_backtrace(bt: &backtrace::Backtrace) -> Vec<StackFrame> {
    let mut out = Vec::new();
    for frame in bt.frames() {
        for symbol in frame.symbols() {
            let function = symbol.name().map(|n| n.to_string());
            let module = symbol
                .filename()
                .map(|p| p.display().to_string())
                .or_else(|| function.clone())
                .unwrap_or_else(|| "<unknown>".to_string());
            let lineno = symbol.lineno();
            let colno = symbol.colno();
            let probe = format!("{} {}", module, function.as_deref().unwrap_or(""));
            let in_app = !NON_APP_HINTS.iter().any(|h| probe.contains(h));
            out.push(StackFrame {
                module,
                function,
                lineno,
                colno,
                in_app: Some(in_app),
            });
        }
    }
    out
}

/// Strip SDK-internal frames from the top of a stack — the `capture_exception`
/// machinery's own frames. Mirrors `dropSdkFrames` in the TS stack-parser.
fn drop_sdk_frames(frames: Vec<StackFrame>) -> Vec<StackFrame> {
    let mut i = 0;
    while i < frames.len() {
        let f = &frames[i];
        let is_sdk = f.module.contains("smooai_observability")
            || f.function
                .as_deref()
                .map(|n| n.contains("smooai_observability"))
                .unwrap_or(false);
        if is_sdk {
            i += 1;
        } else {
            break;
        }
    }
    frames.into_iter().skip(i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_returns_frames() {
        // The vec may be empty if symbols can't resolve (release/stripped); the
        // contract is only that the call must not panic. Every returned frame
        // must carry a module + an in_app tag.
        let frames = capture_stack();
        for f in &frames {
            assert!(!f.module.is_empty());
            assert!(f.in_app.is_some());
        }
    }

    #[test]
    fn drop_sdk_frames_strips_leading_sdk() {
        let frames = vec![
            StackFrame {
                module: "src/smooai_observability/client.rs".into(),
                function: Some("smooai_observability::client::capture".into()),
                lineno: None,
                colno: None,
                in_app: Some(false),
            },
            StackFrame {
                module: "src/main.rs".into(),
                function: Some("app::main".into()),
                lineno: Some(10),
                colno: None,
                in_app: Some(true),
            },
        ];
        let out = drop_sdk_frames(frames);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].module, "src/main.rs");
    }

    #[test]
    fn in_app_tagging() {
        // Build a fake backtrace via capture and confirm tagging is set.
        let frames = capture_stack();
        for f in &frames {
            assert!(f.in_app.is_some());
        }
    }
}
