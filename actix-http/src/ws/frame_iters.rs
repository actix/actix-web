use super::{Frame, Item};

use bytes::Bytes;
use std::str::Chars;

/// Convert binary message content into Frame::Continuation types
///
/// This struct is an iterator over websocket frames
/// with a configurable maximum content size.
/// Original messages that are already within the size
/// limit will be rendered as an iterator over one single
/// binary frame.
/// Original messages that are larger than the size threshold
/// will be converted into an iterator over continuation
/// messages, where the first is a FirstBinary message.
pub struct ContinuationBins<'a> {
    original: &'a [u8],
    step: usize,
    bs_i: usize,
    bs_tot: usize,
    max_frame_content_bytes: usize,
}

impl<'a> ContinuationBins<'a> {
    pub fn new(original: &'a [u8], max_frame_content_bytes: usize) -> Self {
        let bs_tot = original.len();

        Self {
            original,
            step: 0,
            bs_i: 0,
            bs_tot,
            max_frame_content_bytes,
        }
    }
}

impl<'a> Iterator for ContinuationBins<'a> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bs_i >= self.bs_tot {
            None
        } else if self.bs_tot - self.bs_i <= self.max_frame_content_bytes {
            if self.step == 0 {
                // if there are fewer than max bytes remaining to send and
                // we haven't sent anything yet, no continuation frame needed
                self.bs_i += self.max_frame_content_bytes;
                Some(Frame::Binary(Bytes::copy_from_slice(&self.original)))
            } else {
                // otherwise if there are fewer than max bytes remaining to send and
                // we've already sent something, we send a final frame
                let here = self.bs_i;
                self.bs_i += self.max_frame_content_bytes;
                Some(Frame::Continuation(Item::Last(Bytes::copy_from_slice(
                    &self.original[here..self.bs_tot],
                ))))
            }
        } else {
            let item = if self.step == 0 {
                Item::FirstBinary(Bytes::copy_from_slice(
                    &self.original[self.bs_i..self.bs_i + self.max_frame_content_bytes],
                ))
            } else {
                Item::Continue(Bytes::copy_from_slice(
                    &self.original[self.bs_i..self.bs_i + self.max_frame_content_bytes],
                ))
            };
            self.step += 1;
            self.bs_i += self.max_frame_content_bytes;

            Some(Frame::Continuation(item))
        }
    }
}

/// Convert text message content into Frame::Continuation types
///
/// This struct is an iterator over websocket frames
/// with a configurable maximum content size.
/// Original messages that are already within the size
/// limit will be rendered as an iterator over one single
/// text frame.
/// Original messages that are larger than the size threshold
/// will be converted into an iterator over continuation
/// frames, where the first is a FirstText message.
/// Note that for text frames, the maximum content size is
/// fuzzy -- the actual content size may exceed the configured
/// maximum content size by up to 7 bytes, depending on UTF-8
/// encoding of the text string.
pub struct ContinuationTexts<'a> {
    original: Chars<'a>,
    step: usize,
    bs_i: usize,
    bs_tot: usize,
    max_frame_content_bytes: usize,
}

impl<'a> ContinuationTexts<'a> {
    pub fn new(original: Chars<'a>, max_frame_content_bytes: usize) -> Self {
        let bs_tot = original.as_str().len();

        Self {
            original,
            step: 0,
            bs_i: 0,
            bs_tot,
            max_frame_content_bytes,
        }
    }
}

impl<'a> Iterator for ContinuationTexts<'a> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bs_i >= self.bs_tot {
            None
        } else if self.bs_tot - self.bs_i <= self.max_frame_content_bytes {
            let bs = Bytes::copy_from_slice(self.original.as_str().as_bytes());
            self.bs_i += self.max_frame_content_bytes;
            let frm = if self.step == 0 {
                // if there are fewer than max bytes remaining to send and
                // we haven't sent anything yet, no continuation frame needed
                Frame::Text(bs)
            } else {
                // otherwise if there are fewer than max bytes remaining to send and
                // we've already sent something, we send a final frame
                Frame::Continuation(Item::Last(bs))
            };
            Some(frm)
        } else {
            let mut s = String::new();
            let mut temp_i: usize = 0;
            while temp_i < self.max_frame_content_bytes {
                let c = self.original.next();
                if let Some(c) = c {
                    temp_i += c.len_utf8();
                    self.bs_i += c.len_utf8();
                    s.push(c);
                } else {
                    self.bs_i = self.bs_tot;
                    break;
                }
            }
            let item = if self.step == 0 {
                Item::FirstText(Bytes::copy_from_slice(s.as_bytes()))
            } else {
                Item::Continue(Bytes::copy_from_slice(s.as_bytes()))
            };
            self.step += 1;

            Some(Frame::Continuation(item))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_continuation_bins() {
        // render a single Frame::Binary when max size is greater than the payload len
        let mut bins = ContinuationBins::new(b"one two three", 100);
        assert_eq!(bins.next(), Some(Frame::Binary("one two three".into())));

        let mut bins = ContinuationBins::new(b"one two three", 4);
        assert_eq!(
            bins.next(),
            Some(Frame::Continuation(Item::FirstBinary("one ".into())))
        );
        assert_eq!(
            bins.next(),
            Some(Frame::Continuation(Item::Continue("two ".into())))
        );
        assert_eq!(
            bins.next(),
            Some(Frame::Continuation(Item::Continue("thre".into())))
        );
        assert_eq!(
            bins.next(),
            Some(Frame::Continuation(Item::Last("e".into())))
        );
    }

    #[test]
    fn test_continuation_texts() {
        // render a single Frame::Binary when max size is greater than the payload len
        let mut texts = ContinuationTexts::new("one two three".chars(), 100);
        assert_eq!(texts.next(), Some(Frame::Text("one two three".into())));

        let mut texts = ContinuationTexts::new("one two three".chars(), 4);
        assert_eq!(
            texts.next(),
            Some(Frame::Continuation(Item::FirstText("one ".into())))
        );
        assert_eq!(
            texts.next(),
            Some(Frame::Continuation(Item::Continue("two ".into())))
        );
        assert_eq!(
            texts.next(),
            Some(Frame::Continuation(Item::Continue("thre".into())))
        );
        assert_eq!(
            texts.next(),
            Some(Frame::Continuation(Item::Last("e".into())))
        );

        let mut snowmen = ContinuationTexts::new("⛄⛄⛄".chars(), 5);
        assert_eq!(
            snowmen.next(),
            Some(Frame::Continuation(Item::FirstText("⛄⛄".into())))
        );
        assert_eq!(
            snowmen.next(),
            Some(Frame::Continuation(Item::Last("⛄".into())))
        );
    }
}
