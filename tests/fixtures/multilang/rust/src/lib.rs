//! Fixture crate root for the multilang acceptance fixture.

use crate::util::{Helper, make_helper};

/// Greets a name.
pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

pub struct Engine {
    pub helper: Helper,
}

pub fn build_engine() -> Engine {
    Engine {
        helper: make_helper("core"),
    }
}
