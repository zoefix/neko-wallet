//! A single-line text field.
//!
//! Hand-rolled rather than pulled from a crate because the password variant
//! must never place its contents in a type we cannot wipe, and because we want
//! masking, not generic editing.

use unicode_segmentation::UnicodeSegmentation;
use zeroize::Zeroizing;

pub struct Field {
    value: Zeroizing<String>,
    pub masked: bool,
}

impl Field {
    pub fn new(masked: bool) -> Self {
        Self {
            value: Zeroizing::new(String::new()),
            masked,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn push(&mut self, c: char) {
        if !c.is_control() {
            self.value.push(c);
        }
    }

    pub fn backspace(&mut self) {
        let mut g: Vec<&str> = self.value.graphemes(true).collect();
        g.pop();
        self.value = Zeroizing::new(g.concat());
    }

    pub fn clear(&mut self) {
        self.value = Zeroizing::new(String::new());
    }

    /// What to draw. Masked fields never render their contents.
    pub fn display(&self) -> String {
        if self.masked {
            "*".repeat(self.value.graphemes(true).count())
        } else {
            self.value.to_string()
        }
    }
}

impl std::fmt::Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Field")
            .field("masked", &self.masked)
            .field("value", &"[redacted]")
            .finish()
    }
}
