//! In-place MP3 gain by rewriting the per-granule `global_gain` field.
//!
//! `global_gain` is an 8-bit scale exponent in the Layer III side info. The
//! dequantiser multiplies the spectrum by 2^((gg-210)/4), so changing it changes
//! loudness while every Huffman payload, frame length and file offset stays
//! exactly the same. That is what makes it usable inside a VPK.

const BR_MPEG1: [u32; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
const BR_MPEG2: [u32; 16] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
const SR_MPEG1: [u32; 4] = [44100, 48000, 32000, 0];
const SR_MPEG2: [u32; 4] = [22050, 24000, 16000, 0];
const SR_MPEG25: [u32; 4] = [11025, 12000, 8000, 0];

fn get_bits(d: &[u8], pos: usize, len: usize) -> Option<u32> {
    let mut v = 0u32;
    for i in 0..len {
        let bit = pos + i;
        let byte = *d.get(bit / 8)?;
        v = (v << 1) | (((byte >> (7 - (bit % 8))) & 1) as u32);
    }
    Some(v)
}

fn set_bits(d: &mut [u8], pos: usize, len: usize, val: u32) {
    for i in 0..len {
        let bit = pos + i;
        if bit / 8 >= d.len() {
            return;
        }
        let shift = 7 - (bit % 8);
        let mask = 1u8 << shift;
        let on = (val >> (len - 1 - i)) & 1 == 1;
        if on {
            d[bit / 8] |= mask;
        } else {
            d[bit / 8] &= !mask;
        }
    }
}

fn id3_end(d: &[u8]) -> usize {
    if d.len() >= 10 && &d[0..3] == b"ID3" {
        let sz = ((d[6] as usize) << 21) | ((d[7] as usize) << 14) | ((d[8] as usize) << 7) | d[9] as usize;
        let mut end = 10 + sz;
        if d[5] & 0x10 != 0 {
            end += 10; // footer
        }
        return end.min(d.len());
    }
    0
}

#[derive(Clone, Copy)]
struct Header {
    mpeg1: bool,
    mono: bool,
    granules: usize,
    channels: usize,
    frame_len: usize,
    side_len: usize,
}

fn header(d: &[u8], at: usize) -> Option<Header> {
    if at + 4 > d.len() {
        return None;
    }
    let b = &d[at..at + 4];
    if b[0] != 0xFF || (b[1] & 0xE0) != 0xE0 {
        return None;
    }
    let ver = (b[1] >> 3) & 3; // 3 = MPEG1, 2 = MPEG2, 0 = MPEG2.5, 1 = reserved
    let layer = (b[1] >> 1) & 3; // 1 = Layer III
    if ver == 1 || layer != 1 {
        return None;
    }
    let mpeg1 = ver == 3;
    let br_idx = (b[2] >> 4) as usize;
    let sr_idx = ((b[2] >> 2) & 3) as usize;
    let pad = ((b[2] >> 1) & 1) as usize;
    let mono = (b[3] >> 6) == 3;
    let br = if mpeg1 { BR_MPEG1[br_idx] } else { BR_MPEG2[br_idx] };
    let sr = match ver {
        3 => SR_MPEG1[sr_idx],
        2 => SR_MPEG2[sr_idx],
        _ => SR_MPEG25[sr_idx],
    };
    if br == 0 || sr == 0 {
        return None;
    }
    let frame_len = if mpeg1 {
        (144 * br as usize * 1000 / sr as usize) + pad
    } else {
        (72 * br as usize * 1000 / sr as usize) + pad
    };
    let side_len = if mpeg1 {
        if mono { 17 } else { 32 }
    } else if mono {
        9
    } else {
        17
    };
    Some(Header {
        mpeg1,
        mono,
        granules: if mpeg1 { 2 } else { 1 },
        channels: if mono { 1 } else { 2 },
        frame_len,
        side_len,
    })
}

/// Walk the side info and return the bit offset of each `global_gain`.
fn gains(d: &[u8], at: usize, h: Header) -> Option<Vec<usize>> {
    let start = at + 4;
    if start + h.side_len > d.len() {
        return None;
    }
    let mut bit = start * 8;
    bit += 9; // main_data_begin
    bit += if h.mpeg1 {
        if h.mono { 5 } else { 3 }
    } else if h.mono {
        1
    } else {
        2
    };
    if h.mpeg1 {
        bit += 4 * h.channels; // scfsi
    }
    let side_end = start * 8 + h.side_len * 8;
    let mut out = Vec::with_capacity(h.granules * h.channels);
    for _g in 0..h.granules {
        for _c in 0..h.channels {
            let _len = get_bits(d, bit, 12)?; // part2_3_length (lives in main data, not here)
            let big = get_bits(d, bit + 12, 9)?; // big_values
            let _gg = get_bits(d, bit + 21, 8)?;
            if big > 288 {
                return None;
            }
            out.push(bit + 21);
            // walk the rest of this granule header so the next granule starts correctly
            bit += 12 + 9 + 8; // part2_3_length, big_values, global_gain
            bit += if h.mpeg1 { 4 } else { 9 }; // scalefac_compress
            let ws = get_bits(d, bit, 1)?;
            bit += 1;
            if ws == 1 {
                bit += 2 + 1 + 10 + 9; // block_type, mixed_block, table_select[2], subblock_gain[3]
            } else {
                bit += 15 + 4 + 3; // table_select[3], region0_count, region1_count
            }
            bit += if h.mpeg1 { 1 } else { 0 }; // preflag
            bit += 1 + 1; // scalefac_scale, count1table_select
            let _ = big;
        }
    }
    if bit > side_end {
        return None; // walk overshot the side info: layout not what we expect
    }
    Some(out)
}

fn apply(d: &[u8], f: impl Fn(u8) -> u8) -> Option<Vec<u8>> {
    let mut out = d.to_vec();
    let mut at = id3_end(d);
    let mut frames = 0usize;
    let mut edited = 0usize;
    while at + 4 <= d.len() {
        // ID3v1 / APE tail: stop
        if &d[at..at + 3] == b"TAG" || &d[at..at + 4] == b"APET" {
            break;
        }
        let Some(h) = header(d, at) else {
            if frames > 0 {
                break;
            }
            at += 1; // resync over junk between frames
            continue;
        };
        if h.frame_len == 0 || at + h.frame_len > d.len() {
            break;
        }
        if let Some(pos) = gains(d, at, h) {
            for p in pos {
                let Some(g) = get_bits(d, p, 8) else { break };
                set_bits(&mut out, p, 8, f(g as u8) as u32);
                edited += 1;
            }
        } else {
            return None; // inconsistent side info: do not touch the file
        }
        frames += 1;
        at += h.frame_len;
    }
    if frames == 0 || edited == 0 {
        return None;
    }
    Some(out)
}

/// Scale amplitude by `factor` (0.1 .. 4.0), length-preserving.
pub fn scale(data: &[u8], factor: f32) -> Option<Vec<u8>> {
    if !(factor > 0.0) || !factor.is_finite() {
        return None;
    }
    let delta = (4.0 * factor.log2()).round() as i32; // 4 steps per octave
    apply(data, |gg| (gg as i32 + delta).clamp(0, 255) as u8)
}

/// Drop the amplitude to the noise floor of the format.
pub fn silence(data: &[u8]) -> Option<Vec<u8>> {
    apply(data, |_| 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One MPEG-1 Layer III stereo frame with zeroed side info and body.
    fn frame(gg_left: u8, gg_right: u8) -> Vec<u8> {
        // 0xFF 0xFB: MPEG1, Layer III, no CRC. 0x90: 256 kbps, 44.1 kHz, no pad.
        // 0x00: stereo.
        let mut f = vec![0xFF, 0xFB, 0x90, 0x00];
        f.extend(std::iter::repeat(0u8).take(32)); // side info
        let h = header(&f, 0).unwrap();
        let positions = gains(&f, 0, h).unwrap();
        set_bits(&mut f, positions[0], 8, gg_left as u32);
        set_bits(&mut f, positions[1], 8, gg_right as u32);
        f.extend(std::iter::repeat(0u8).take(h.frame_len - 4 - 32));
        f
    }

    #[test]
    fn scale_changes_gain_and_keeps_length() {
        let f = frame(210, 210);
        let out = scale(&f, 0.5).unwrap();
        assert_eq!(out.len(), f.len());
        let h = header(&out, 0).unwrap();
        let pos = gains(&out, 0, h).unwrap();
        assert_eq!(get_bits(&out, pos[0], 8).unwrap(), 206); // -4 steps = half
        assert_eq!(get_bits(&out, pos[1], 8).unwrap(), 206);
    }

    #[test]
    fn silence_sets_zero_and_rejects_garbage() {
        let f = frame(200, 200);
        let out = silence(&f).unwrap();
        let h = header(&out, 0).unwrap();
        for p in gains(&out, 0, h).unwrap() {
            assert_eq!(get_bits(&out, p, 8).unwrap(), 0);
        }
        assert!(scale(&[0u8; 128], 2.0).is_none());
    }

    #[test]
    fn short_block_granule_keeps_the_walk_aligned() {
        let mut f = frame(210, 210);
        let h = header(&f, 0).unwrap();
        // window_switching_flag of granule 0, channel 0 sits after its 29 fixed bits + 4 scalefac bits
        let ws_bit = 4 * 8 + 9 + 3 + 8 + 29 + 4;
        set_bits(&mut f, ws_bit, 1, 1);
        let positions = gains(&f, 0, h).unwrap();
        assert_eq!(positions.len(), 4);
        assert_eq!(positions[1], 4 * 8 + 79 + 21); // channel 1 starts right after the short-block header
    }

    #[test]
    fn clamps_instead_of_wrapping() {
        let f = frame(254, 3);
        let out = scale(&f, 4.0).unwrap();
        let h = header(&out, 0).unwrap();
        let pos = gains(&out, 0, h).unwrap();
        assert_eq!(get_bits(&out, pos[0], 8).unwrap(), 255);
        let low = scale(&f, 0.1).unwrap();
        let h2 = header(&low, 0).unwrap();
        let p2 = gains(&low, 0, h2).unwrap();
        assert_eq!(get_bits(&low, p2[1], 8).unwrap(), 0);
    }
}
