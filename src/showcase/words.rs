//! What a version becomes (the names stand-ins come from live in
//! `universe.rs`).

/// What a version becomes, by its shape's width (`0.9.5` → `4.2.0`).
#[rustfmt::skip]
pub const VERSIONS: &[&str] = &[
    // 42, NCC-1701, 2001, π, and Lost's numbers
    "4.2", "1.0", "3.1", "2.0", "4.8",
    "42.0", "17.1", "20.1", "3.14", "4.15",
    "4.2.0", "1.7.1", "9.7.5", "3.1.4", "4.8.1", "2.0.1", "7.4.2", "6.0.2", "8.1.5", "5.4.2",
    "42.0.0", "4.8.15", "3.14.1", "1.70.1", "16.4.2", "2.42.0", "20.0.1", "17.0.1",
    "1.701.0", "2.42.17", "3.14.15", "16.23.4", "42.0.17", 
    "42.42.42", "1.701.42", "16.23.42", "2001.4.2", "3.14.159",
    "0.42.1701", "4.8.15.16", "2001.1.42",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_numbers_and_dots() {
        for v in VERSIONS {
            assert!(v.bytes().all(|b| b.is_ascii_digit() || b == b'.'), "{v}");
        }
    }
}
