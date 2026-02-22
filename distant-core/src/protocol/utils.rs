/// Used purely for skipping serialization of values that are false by default.
#[inline]
pub const fn is_false(value: &bool) -> bool {
    !*value
}

/// Used purely for skipping serialization of values that are 1 by default.
#[inline]
pub const fn is_one(value: &usize) -> bool {
    *value == 1
}

/// Used to provide a default serde value of 1.
#[inline]
pub const fn one() -> usize {
    1
}
