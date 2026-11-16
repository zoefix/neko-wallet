//! Every chain's HTTP traffic, in one place.
//!
//! Each chain crate used to build its own client and its own retry loop, and
//! each loop knew a different subset of what a public node does when it is
//! busy. A wallet list asks eighteen chains about every wallet at once, so the
//! one that knew least - TRON's history, which retried nothing - decided what
//! the screen looked like. This crate is the part they now share:
//!
//! * **one connection pool** for the whole process ([`http::client`]);
//! * **a throttle per host**, and per key on hosts that take one, which spaces
//!   requests out before a node has to say so and backs off further when it
//!   does ([`throttle`]);
//! * **one retry policy**, which waits on what is worth waiting for and never
//!   on a decision the node has already made ([`retry`], [`classify`]);
//! * **failover** to built-in public nodes when the default refuses, fails or
//!   has a queue behind it - used only when nobody configured a node for
//!   that chain ([`Service::for_chain`]);
//! * **key rotation**, for TronGrid's per-key limits ([`keys`]).
//!
//! What this crate never does is resend a transaction that may have reached a
//! node. See [`Policy::Submit`].

pub mod classify;
pub mod http;
pub mod keys;
pub mod retry;
mod service;
pub mod throttle;

#[cfg(test)]
mod testserver;

pub use classify::{Classifier, Verdict};
pub use keys::KeyRing;
pub use retry::Retry;
pub use service::{NetError, Policy, Reply, Request, Service};
