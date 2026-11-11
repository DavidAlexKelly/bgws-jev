//! HLA 4 (IEEE 1516-2025) through the Federate Protocol, with the RPR FOM
//! (docs/design/INTEROP_PLAN.md, phase 3).
//!
//! - [`proto`]: the Federate Protocol's call and callback messages, generated
//!   from the IEEE protos (`proto/README.md`).
//! - [`session`]: the Federate Protocol session layer.
//! - [`rti`]: a typed RTI ambassador over a session.
//! - [`encoding`], [`rpr`]: the HLA data encoding and the RPR FOM structures.
//! - [`link`]: a game in a federation, publishing and taking in.
//! - [`fom`]: a federation's object model, read from its FOM modules.
//! - [`mock`]: a Federate Protocol server with a small RTI, for tests.

/// Generated from IEEE 1516.1-2025's Federate Protocol protos.
/// Reprinted with permission from IEEE 1516.1(TM)-2025.
#[allow(clippy::all, clippy::pedantic, missing_docs, rustdoc::all)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/rti1516_2025.fedpro.rs"));
}

pub mod encoding;
pub mod fom;
pub mod link;
pub mod mock;
pub mod rpr;
pub mod rti;
pub mod session;
