//! Event handler trait for processing events from libscoop operations.
//!
//! Implementations translate [`Event`]s into user-facing output.
//! The event loop (`hok::eventloop`) calls [`EventHandler::handle`] for each
//! event emitted during package sync operations.
//!
//! # Example (in hok CLI crate)
//!
//! ```ignore
//! use libscoop::{Event, EventHandler, Transaction};
//!
//! struct MyHandler;
//!
//! impl EventHandler for MyHandler {
//!     fn handle(&mut self, event: &Event) {
//!         match event {
//!             Event::PackageResolveStart => println!("Resolving..."),
//!             Event::PackageResolveDone => println!("Done."),
//!             _ => {}
//!         }
//!     }
//! }
//! ```

use crate::Event;

/// Trait for handling events emitted during libscoop operations.
///
/// Implementations should translate events into user-facing output
/// (e.g., terminal messages, progress bars, or a GUI).
///
/// The trait requires `Send` so it can be used across threads
/// (the event loop runs on a dedicated thread).
pub trait EventHandler: Send + 'static {
    /// Handle a single event.
    ///
    /// Called for each event emitted during package sync operations.
    fn handle(&mut self, event: &Event);

    /// Called when the event stream has been exhausted.
    ///
    /// The default implementation does nothing.
    fn on_finished(&mut self) {}
}
