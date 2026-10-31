//! The files the core and the gate hand each other in the gate's configuration directory,
//! and the shapes they share: the certificates a route's upstream may present and a token
//! held only as its hash.

pub mod gate;

mod pins;
mod shape;
mod token;
mod unreadable;

pub use pins::Pins;
pub use token::TokenHash;
pub use unreadable::Unreadable;
