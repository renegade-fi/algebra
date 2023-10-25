/// Represents metadata to be appended to an object's serialization. For
/// example, when serializing elliptic curve points, one can
/// use a `Flag` to represent whether the serialization is the point
/// at infinity, or whether the `y` coordinate is positive or not.
/// These bits will be appended to the end of the point's serialization,
/// or included in a new byte, depending on space available.
///
/// This is meant to be provided to `CanonicalSerializeWithFlags` and
/// `CanonicalDeserializeWithFlags`
pub trait Flags: Default + Clone + Copy + Sized {
    /// The number of bits required to encode `Self`.
    /// This should be at most 8.
    const BIT_SIZE: usize;

    // Returns a bit mask corresponding to `self`.
    // For example, if `Self` contains two variants, there are just two possible
    // bit masks: `0` and `1 << 7`.
    fn u8_bitmask(&self) -> u8;

    // Tries to read `Self` from `value`. Should return `None` if the
    // `Self::BIT_SIZE` most-significant bits of `value` do not correspond to
    // those generated by `u8_bitmask`.
    //
    // That is, this method ignores all but the top `Self::BIT_SIZE` bits, and
    // decides whether these top bits correspond to a bitmask output by
    // `u8_bitmask`.
    fn from_u8(value: u8) -> Option<Self>;

    // Convenience method that reads `Self` from `value`, just like `Self::from_u8`,
    // but additionally zeroes out the bits corresponding to the resulting flag
    // in `value`. If `Self::from_u8(*value)` would return `None`, then this
    // method should *not* modify `value`.
    fn from_u8_remove_flags(value: &mut u8) -> Option<Self> {
        let flags = Self::from_u8(*value);
        if let Some(f) = flags {
            *value &= !f.u8_bitmask();
        }
        flags
    }
}

/// Flags to be encoded into the serialization.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct EmptyFlags;

impl Flags for EmptyFlags {
    const BIT_SIZE: usize = 0;

    #[cfg_attr(not(feature = "bin-opt"), inline)]
    fn u8_bitmask(&self) -> u8 {
        0
    }

    #[cfg_attr(not(feature = "bin-opt"), inline)]
    fn from_u8(_: u8) -> Option<Self> {
        Some(EmptyFlags)
    }
}
