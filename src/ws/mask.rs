//! This is code from [Tungstenite project](https://github.com/snapview/tungstenite-rs)
use std::cmp::min;
use std::mem::uninitialized;
use std::ptr::copy_nonoverlapping;

/// Mask/unmask a frame.
#[inline]
pub fn apply_mask(buf: &mut [u8], mask: &[u8; 4]) {
    apply_mask_fast32(buf, mask)
}

/// A safe unoptimized mask application.
#[inline]
#[allow(dead_code)]
fn apply_mask_fallback(buf: &mut [u8], mask: &[u8; 4]) {
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte ^= mask[i & 3];
    }
}

/// Faster version of `apply_mask()` which operates on 4-byte blocks.
#[inline]
#[allow(dead_code)]
fn apply_mask_fast32(buf: &mut [u8], mask: &[u8; 4]) {
    // TODO replace this with read_unaligned() as it stabilizes.
    let mask_u32 = unsafe {
        let mut m: u32 = uninitialized();
        #[allow(trivial_casts)]
        copy_nonoverlapping(mask.as_ptr(), &mut m as *mut _ as *mut u8, 4);
        m
    };

    let mut ptr = buf.as_mut_ptr();
    let mut len = buf.len();

    // Possible first unaligned block.
    let head = min(len, (4 - (ptr as usize & 3)) & 3);
    let mask_u32 = if head > 0 {
        unsafe {
            xor_mem(ptr, mask_u32, head);
            ptr = ptr.offset(head as isize);
        }
        len -= head;
        if cfg!(target_endian = "big") {
            mask_u32.rotate_left(8 * head as u32)
        } else {
            mask_u32.rotate_right(8 * head as u32)
        }
    } else {
        mask_u32
    };

    if len > 0 {
        debug_assert_eq!(ptr as usize % 4, 0);
    }

    // Properly aligned middle of the data.
    while len > 4 {
        unsafe {
            *(ptr as *mut u32) ^= mask_u32;
            ptr = ptr.offset(4);
            len -= 4;
        }
    }

    // Possible last block.
    if len > 0 {
        unsafe { xor_mem(ptr, mask_u32, len); }
    }
}

#[inline]
// TODO: copy_nonoverlapping here compiles to call memcpy. While it is not so inefficient,
// it could be done better. The compiler does not see that len is limited to 3.
unsafe fn xor_mem(ptr: *mut u8, mask: u32, len: usize) {
    let mut b: u32 = uninitialized();
    #[allow(trivial_casts)]
    copy_nonoverlapping(ptr, &mut b as *mut _ as *mut u8, len);
    b ^= mask;
    #[allow(trivial_casts)]
    copy_nonoverlapping(&b as *const _ as *const u8, ptr, len);
}

#[cfg(test)]
mod tests {
 use super::{apply_mask_fallback, apply_mask_fast32};

    #[test]
    fn test_apply_mask() {
        let mask = [
            0x6d, 0xb6, 0xb2, 0x80,
        ];
        let unmasked = vec![
            0xf3, 0x00, 0x01, 0x02,  0x03, 0x80, 0x81, 0x82,
            0xff, 0xfe, 0x00, 0x17,  0x74, 0xf9, 0x12, 0x03,
        ];

        // Check masking with proper alignment.
        {
            let mut masked = unmasked.clone();
            apply_mask_fallback(&mut masked, &mask);

            let mut masked_fast = unmasked.clone();
            apply_mask_fast32(&mut masked_fast, &mask);

            assert_eq!(masked, masked_fast);
        }

        // Check masking without alignment.
        {
            let mut masked = unmasked.clone();
            apply_mask_fallback(&mut masked[1..], &mask);

            let mut masked_fast = unmasked.clone();
            apply_mask_fast32(&mut masked_fast[1..], &mask);

            assert_eq!(masked, masked_fast);
        }
    }
}
