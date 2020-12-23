//! This is code from [Tungstenite project](https://github.com/snapview/tungstenite-rs)
#![allow(clippy::cast_ptr_alignment)]
use std::ptr::copy_nonoverlapping;
use std::slice;

// Holds a slice guaranteed to be shorter than 8 bytes
struct ShortSlice<'a> {
    inner: &'a mut [u8],
}

impl<'a> ShortSlice<'a> {
    /// # Safety
    /// Given slice must be shorter than 8 bytes.
    unsafe fn new(slice: &'a mut [u8]) -> Self {
        // Sanity check for debug builds
        debug_assert!(slice.len() < 8);
        ShortSlice { inner: slice }
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

/// Faster version of `apply_mask()` which operates on 8-byte blocks.
#[inline]
#[allow(clippy::cast_lossless)]
pub(crate) fn apply_mask(buf: &mut [u8], mask_u32: u32) {
    // Extend the mask to 64 bits
    let mut mask_u64 = ((mask_u32 as u64) << 32) | (mask_u32 as u64);
    // Split the buffer into three segments
    let (head, mid, tail) = align_buf(buf);

    // Initial unaligned segment
    let head_len = head.len();
    if head_len > 0 {
        xor_short(head, mask_u64);
        if cfg!(target_endian = "big") {
            mask_u64 = mask_u64.rotate_left(8 * head_len as u32);
        } else {
            mask_u64 = mask_u64.rotate_right(8 * head_len as u32);
        }
    }
    // Aligned segment
    for v in mid {
        *v ^= mask_u64;
    }
    // Final unaligned segment
    if tail.len() > 0 {
        xor_short(tail, mask_u64);
    }
}

// TODO: copy_nonoverlapping here compiles to call memcpy. While it is not so
// inefficient, it could be done better. The compiler does not understand that
// a `ShortSlice` must be smaller than a u64.
#[inline]
#[allow(clippy::needless_pass_by_value)]
fn xor_short(buf: ShortSlice<'_>, mask: u64) {
    // SAFETY: we know that a `ShortSlice` fits in a u64
    unsafe {
        let (ptr, len) = (buf.inner.as_mut_ptr(), buf.len());
        let mut b: u64 = 0;
        #[allow(trivial_casts)]
        copy_nonoverlapping(ptr, &mut b as *mut _ as *mut u8, len);
        b ^= mask;
        #[allow(trivial_casts)]
        copy_nonoverlapping(&b as *const _ as *const u8, ptr, len);
    }
}

/// # Safety
/// Caller must ensure the buffer has the correct size and alignment.
#[inline]
unsafe fn cast_slice(buf: &mut [u8]) -> &mut [u64] {
    // Assert correct size and alignment in debug builds
    debug_assert!(buf.len().trailing_zeros() >= 3);
    debug_assert!((buf.as_ptr() as usize).trailing_zeros() >= 3);

    slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u64, buf.len() >> 3)
}

// Splits a slice into three parts: an unaligned short head and tail, plus an aligned
// u64 mid section.
#[inline]
fn align_buf(buf: &mut [u8]) -> (ShortSlice<'_>, &mut [u64], ShortSlice<'_>) {
    let start_ptr = buf.as_ptr() as usize;
    let end_ptr = start_ptr + buf.len();

    // Round *up* to next aligned boundary for start
    let start_aligned = (start_ptr + 7) & !0x7;
    // Round *down* to last aligned boundary for end
    let end_aligned = end_ptr & !0x7;

    if end_aligned >= start_aligned {
        // We have our three segments (head, mid, tail)
        let (tmp, tail) = buf.split_at_mut(end_aligned - start_ptr);
        let (head, mid) = tmp.split_at_mut(start_aligned - start_ptr);

        // SAFETY: we know the middle section is correctly aligned, and the outer
        // sections are smaller than 8 bytes
        unsafe {
            (
                ShortSlice::new(head),
                cast_slice(mid),
                ShortSlice::new(tail),
            )
        }
    } else {
        // We didn't cross even one aligned boundary!

        // SAFETY: The outer sections are smaller than 8 bytes
        unsafe { (ShortSlice::new(buf), &mut [], ShortSlice::new(&mut [])) }
    }
}

#[cfg(test)]
mod tests {
    use super::apply_mask;

    /// A safe unoptimized mask application.
    fn apply_mask_fallback(buf: &mut [u8], mask: &[u8; 4]) {
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte ^= mask[i & 3];
        }
    }

    #[test]
    fn test_apply_mask() {
        let mask = [0x6d, 0xb6, 0xb2, 0x80];
        let mask_u32 = u32::from_le_bytes(mask);

        let unmasked = vec![
            0xf3, 0x00, 0x01, 0x02, 0x03, 0x80, 0x81, 0x82, 0xff, 0xfe, 0x00, 0x17,
            0x74, 0xf9, 0x12, 0x03,
        ];

        // Check masking with proper alignment.
        {
            let mut masked = unmasked.clone();
            apply_mask_fallback(&mut masked, &mask);

            let mut masked_fast = unmasked.clone();
            apply_mask(&mut masked_fast, mask_u32);

            assert_eq!(masked, masked_fast);
        }

        // Check masking without alignment.
        {
            let mut masked = unmasked.clone();
            apply_mask_fallback(&mut masked[1..], &mask);

            let mut masked_fast = unmasked;
            apply_mask(&mut masked_fast[1..], mask_u32);

            assert_eq!(masked, masked_fast);
        }
    }
}
