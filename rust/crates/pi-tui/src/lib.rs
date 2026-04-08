pub mod fuzzy;

pub use fuzzy::{FuzzyMatch, fuzzy_filter, fuzzy_match};

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("tui migration pending")]
    Pending,
}
