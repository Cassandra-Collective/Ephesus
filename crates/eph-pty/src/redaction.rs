//! Redact exact known secret values before PTY bytes reach a sink (ADR-0010).

/// The visible replacement required by ADR-0010.
pub const MASK: &[u8] = "•••eph-masked•••".as_bytes();

/// The shortest value the reference redactor masks, in UTF-16 code units.
pub const MIN_SECRET_LEN: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Plain,
    Start,
    Covered,
}

/// One filter per stream. Non-secret bytes are preserved without UTF-8 decoding.
///
/// Supply the current broker values at each call. The filter retains only a
/// suffix that could still complete a match, plus its existing mask marks. A
/// completed match remains masked if the supplied value changes before flush.
#[derive(Default)]
pub struct Redactor {
    pending: Vec<u8>,
    marks: Vec<Mark>,
}

impl Redactor {
    /// Consume a chunk and return the bytes that can already be emitted safely.
    pub fn push(&mut self, chunk: &[u8], values: &[&str]) -> Vec<u8> {
        self.pending.extend_from_slice(chunk);
        self.marks.resize(self.pending.len(), Mark::Plain);
        self.take_ready(values, false)
    }

    /// Emit any pending bytes at stream end, masking all completed matches.
    pub fn flush(&mut self, values: &[&str]) -> Vec<u8> {
        self.take_ready(values, true)
    }

    fn take_ready(&mut self, values: &[&str], at_end: bool) -> Vec<u8> {
        let secrets: Vec<_> = values
            .iter()
            .filter(|value| value.encode_utf16().count() >= MIN_SECRET_LEN)
            .map(|value| value.as_bytes())
            .collect();
        let mut ready = self.pending.len();
        let mut covered_until = 0;

        // Examine all starts before emitting. A short match and the start of
        // a longer match can occupy the same bytes, and ranges can overlap.
        for (start, mark) in self.marks.iter_mut().enumerate() {
            let remaining = &self.pending[start..];
            let mut longest = 0;
            for secret in &secrets {
                if remaining.starts_with(secret) {
                    longest = longest.max(secret.len());
                } else if !at_end && secret.starts_with(remaining) {
                    ready = ready.min(start);
                }
            }
            if start < covered_until {
                *mark = Mark::Covered;
            } else if longest > 0 && *mark == Mark::Plain {
                *mark = Mark::Start;
            }
            covered_until = covered_until.max(start + longest);
        }

        let mut output = Vec::with_capacity(ready);
        for (byte, mark) in self.pending.drain(..ready).zip(self.marks.drain(..ready)) {
            match mark {
                Mark::Plain => output.push(byte),
                Mark::Start => output.extend_from_slice(MASK),
                Mark::Covered => {}
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(chunks: &[&[u8]], values: &[&str]) -> Vec<u8> {
        let mut redactor = Redactor::default();
        let mut output = Vec::new();
        for chunk in chunks {
            output.extend(redactor.push(chunk, values));
        }
        output.extend(redactor.flush(values));
        output
    }

    #[test]
    fn every_split_of_a_prefix_pair_masks_the_whole_longer_value() {
        let input = b"demo-tail";
        for split in 0..=input.len() {
            assert_eq!(
                drain(&[&input[..split], &input[split..]], &["demo", "demo-tail"]),
                MASK,
                "split at {split}"
            );
        }
    }

    #[test]
    fn overlapping_matches_hide_the_entire_union_at_every_split() {
        let input = b"abcdefgh!";
        let expected = [MASK, b"!"].concat();
        for split in 0..=input.len() {
            assert_eq!(
                drain(&[&input[..split], &input[split..]], &["abcde", "cdefgh"]),
                expected,
                "split at {split}"
            );
        }
    }

    #[test]
    fn adjacent_occurrences_each_have_a_marker_even_with_bytewise_delivery() {
        let chunks = b"demodemo!".chunks(1).collect::<Vec<_>>();
        assert_eq!(drain(&chunks, &["demo"]), [MASK, MASK, b"!"].concat());
    }

    #[test]
    fn plaintext_and_non_utf8_bytes_pass_immediately() {
        let mut redactor = Redactor::default();
        assert_eq!(redactor.push(b"ready> \xff", &["demo"]), b"ready> \xff");
        assert!(redactor.flush(&["demo"]).is_empty());
    }

    #[test]
    fn an_incomplete_prefix_is_released_on_mismatch_or_exit() {
        let mut redactor = Redactor::default();
        assert_eq!(redactor.push(b"prompt: de", &["demo"]), b"prompt: ");
        assert_eq!(redactor.push(b"pth\r\n", &["demo"]), b"depth\r\n");
        assert!(redactor.push(b"dem", &["demo"]).is_empty());
        assert_eq!(redactor.flush(&["demo"]), b"dem");
        assert!(redactor.flush(&["demo"]).is_empty());
    }

    #[test]
    fn rotation_cannot_unmask_a_completed_match_buffered_before_it() {
        let mut redactor = Redactor::default();
        assert!(redactor.push(b"demo", &["demo", "demo-tail"]).is_empty());
        assert_eq!(redactor.flush(&["next"]), MASK);
        assert_eq!(redactor.push(b"next", &["next"]), MASK);
    }

    #[test]
    fn a_new_value_can_mask_already_held_bytes() {
        let mut redactor = Redactor::default();
        assert!(redactor.push(b"dem", &["demo"]).is_empty());
        assert_eq!(redactor.push(b"and", &["demand"]), MASK);
    }

    #[test]
    fn unchanged_reference_floor_includes_utf16_length() {
        assert_eq!(drain(&["a界".as_bytes()], &["", "a界"]), "a界".as_bytes());
        let secret = "🌑🌒";
        let chunks = secret.as_bytes().chunks(1).collect::<Vec<_>>();
        assert_eq!(drain(&chunks, &[secret]), MASK);
    }
}
