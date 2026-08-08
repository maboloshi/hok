//! Structured output channel for session backends.
//!
//! This module is the **output channel** of the library: it defines the
//! structured [`Output`] requests that library code can emit, and the sink
//! mechanism through which a frontend (e.g. the hok CLI) receives them.
//!
//! # Design
//!
//! - **Channel, not renderer**: Rendering is *not* this module's job. A
//!   frontend injects a sink via [`Session::set_output`] and renders each
//!   [`Output`] request in its own UI layer (the hok CLI renders in
//!   `src/output.rs`). Library code must never write to stdout/stderr
//!   directly — use [`Session::output`] instead.
//! - **Silent without a sink**: When no sink is injected, output is silently
//!   dropped. The library itself never produces user-facing output.
//! - **Synchronous**: Requests are forwarded to the sink synchronously (unlike
//!   the async [`Event`][1] bus), so commands that do not run an event loop
//!   (e.g. `ci-auto-pr`, `cache`, `cleanup`) can still emit output safely.
//!
//! [1]: crate::Event

use std::sync::Arc;

/// A single structured output request emitted by library code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Plain informational message (stdout).
    Info(String),
    /// Warning message (stderr).
    Warn(String),
    /// Error message (stderr).
    Error(String),
    /// Operation-completed message.
    Done(String),
    /// "ok" marker, typically paired with [`Output::Progress`].
    Ok,
    /// Inline progress marker, e.g. `Snapshoting manifests ... `.
    Progress {
        /// Action verb, e.g. `Snapshoting`.
        action: String,
        /// Subject, e.g. `manifests`.
        target: String,
    },
    /// Section header.
    Header(String),
    /// Label-value pair, e.g. `Repository: owner/repo`.
    Named {
        /// Label, e.g. `Repository`.
        label: String,
        /// Value, e.g. `owner/repo`.
        value: String,
    },
}

/// Sink consuming [`Output`] requests; injected by the frontend.
pub type OutputSink = Arc<dyn Fn(Output) + Send + Sync>;

/// Handle returned by [`Session::output`] for emitting structured output.
///
/// Each method forwards the corresponding [`Output`] request to the session's
/// sink. When no sink is set, the request is silently dropped.
pub struct OutputHandle<'a> {
    sink: Option<&'a OutputSink>,
}

impl<'a> OutputHandle<'a> {
    /// Create a handle backed by the given sink (if any).
    pub(crate) fn new(sink: Option<&'a OutputSink>) -> Self {
        Self { sink }
    }

    /// Emit an informational message.
    pub fn info(&self, msg: impl Into<String>) {
        self.emit(Output::Info(msg.into()));
    }

    /// Emit a warning message.
    pub fn warn(&self, msg: impl Into<String>) {
        self.emit(Output::Warn(msg.into()));
    }

    /// Emit an error message.
    pub fn error(&self, msg: impl Into<String>) {
        self.emit(Output::Error(msg.into()));
    }

    /// Emit an operation-completed message.
    pub fn done(&self, msg: impl Into<String>) {
        self.emit(Output::Done(msg.into()));
    }

    /// Emit the "ok" marker.
    pub fn ok(&self) {
        self.emit(Output::Ok);
    }

    /// Emit an inline progress marker (paired with [`Self::ok`] /
    /// [`Self::done`]).
    pub fn progress(&self, action: impl Into<String>, target: impl Into<String>) {
        self.emit(Output::Progress {
            action: action.into(),
            target: target.into(),
        });
    }

    /// Emit a section header.
    pub fn header(&self, msg: impl Into<String>) {
        self.emit(Output::Header(msg.into()));
    }

    /// Emit a label-value pair.
    pub fn named(&self, label: impl Into<String>, value: impl Into<String>) {
        self.emit(Output::Named {
            label: label.into(),
            value: value.into(),
        });
    }

    fn emit(&self, output: Output) {
        if let Some(sink) = self.sink {
            sink(output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_sink_receives_structured_output() {
        let root = crate::test_utils::tmpdir("output_sink");
        let session = crate::test_utils::test_session(&root);

        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = {
            let received = Arc::clone(&received);
            Arc::new(move |out: Output| received.lock().unwrap().push(out))
        };
        session.set_output(sink).unwrap();

        session.output().header("Auto PR");
        session.output().named("Repository", "owner/repo");
        session.output().progress("Snapshoting", "manifests");
        session.output().ok();
        session.output().done("done msg");
        session.output().warn("warn msg");
        session.output().error("error msg");
        session.output().info("info msg");

        let got = received.lock().unwrap();
        assert_eq!(got.len(), 8);
        assert_eq!(got[0], Output::Header("Auto PR".into()));
        assert_eq!(
            got[1],
            Output::Named {
                label: "Repository".into(),
                value: "owner/repo".into(),
            }
        );
        assert_eq!(
            got[2],
            Output::Progress {
                action: "Snapshoting".into(),
                target: "manifests".into(),
            }
        );
        assert_eq!(got[3], Output::Ok);
        assert_eq!(got[4], Output::Done("done msg".into()));
        assert_eq!(got[5], Output::Warn("warn msg".into()));
        assert_eq!(got[6], Output::Error("error msg".into()));
        assert_eq!(got[7], Output::Info("info msg".into()));
    }

    #[test]
    fn output_without_sink_is_silent() {
        let root = crate::test_utils::tmpdir("output_silent");
        let session = crate::test_utils::test_session(&root);
        // No sink injected: emitting must neither panic nor crash.
        session.output().info("no crash");
        session.output().progress("Holding", "7zip");
        session.output().ok();
    }

    #[test]
    fn set_output_twice_errors() {
        let root = crate::test_utils::tmpdir("output_twice");
        let session = crate::test_utils::test_session(&root);
        session.set_output(Arc::new(|_| {})).unwrap();
        let err = session.set_output(Arc::new(|_| {})).unwrap_err();
        assert!(matches!(err, crate::Error::OutputAlreadySet));
    }
}
