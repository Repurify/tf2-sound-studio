#[derive(Clone, Copy, Debug)]
pub struct WavInfo {
    pub fmt: u16,
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
    pub data_start: usize,
    pub data_len: usize,
}

fn u16at(d: &[u8], p: usize) -> Option<u16> {
    let b = d.get(p..p + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}
fn u32at(d: &[u8], p: usize) -> Option<u32> {
    let b = d.get(p..p + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub fn info(d: &[u8]) -> Option<WavInfo> {
    if d.len() < 44 || d.get(0..4)? != b"RIFF" || d.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut p = 12usize;
    let mut fmt = None;
    let mut data = None;
    while p + 8 <= d.len() {
        let id = d.get(p..p + 4)?;
        let sz = u32at(d, p + 4)? as usize;
        let body = p + 8;
        if id == b"fmt " {
            if sz < 16 || body + 16 > d.len() {
                return None;
            }
            fmt = Some((
                u16at(d, body)?,
                u16at(d, body + 2)?,
                u32at(d, body + 4)?,
                u16at(d, body + 14)?,
            ));
        } else if id == b"data" {
            let avail = d.len().saturating_sub(body);
            data = Some((body, sz.min(avail)));
            break;
        }
        p = body.checked_add(sz)?.checked_add(sz & 1)?;
    }
    let (fmt_tag, channels, rate, bits) = fmt?;
    let (data_start, data_len) = data?;
    if channels == 0 || rate == 0 || bits == 0 {
        return None;
    }
    if data_start.checked_add(data_len)? > d.len() {
        return None;
    }
    Some(WavInfo {
        fmt: fmt_tag,
        channels,
        rate,
        bits,
        data_start,
        data_len,
    })
}

pub fn is_pcm16(w: &WavInfo) -> bool {
    w.fmt == 1 && w.bits == 16 && (1..=2).contains(&w.channels)
}

/// Silence that actually is silence: 8-bit PCM is unsigned with 0x80 as zero.
pub fn silence(d: &[u8]) -> Option<Vec<u8>> {
    let w = info(d)?;
    if w.fmt != 1 {
        return None; // compressed / float / mp3-in-wav: leave it alone
    }
    let mut out = d.to_vec();
    let fill = if w.bits == 8 { 0x80u8 } else { 0x00u8 };
    out.get_mut(w.data_start..w.data_start + w.data_len)?
        .fill(fill);
    Some(out)
}

/// Returns (new bytes, clipped samples, total samples). None when unsupported.
pub fn gain(d: &[u8], factor: f32) -> Option<(Vec<u8>, usize, usize)> {
    let w = info(d)?;
    if w.fmt != 1 {
        return None;
    }
    let mut out = d.to_vec();
    let mut clipped = 0usize;
    match w.bits {
        16 => {
            let n = w.data_len / 2;
            for i in 0..n {
                let p = w.data_start + i * 2;
                let s = i16::from_le_bytes([*out.get(p)?, *out.get(p + 1)?]);
                let v = (s as f32 * factor).round();
                let v = if v > 32767.0 {
                    clipped += 1;
                    32767i16
                } else if v < -32768.0 {
                    clipped += 1;
                    -32768i16
                } else {
                    v as i16
                };
                out.get_mut(p..p + 2)?.copy_from_slice(&v.to_le_bytes());
            }
            Some((out, clipped, n))
        }
        8 => {
            let n = w.data_len;
            for i in 0..n {
                let p = w.data_start + i;
                let s = *out.get(p)? as f32 - 128.0;
                let v = s * factor + 128.0;
                let v = if v > 255.0 {
                    clipped += 1;
                    255u8
                } else if v < 0.0 {
                    clipped += 1;
                    0u8
                } else {
                    v as u8
                };
                *out.get_mut(p)? = v;
            }
            Some((out, clipped, n))
        }
        _ => None,
    }
}

fn decode16(d: &[u8], w: &WavInfo) -> Option<Vec<Vec<f32>>> {
    let ch = w.channels as usize;
    let frames = w.data_len / (2 * ch);
    let mut out = vec![Vec::with_capacity(frames); ch];
    for i in 0..frames {
        for c in 0..ch {
            let p = w.data_start + (i * ch + c) * 2;
            let s = i16::from_le_bytes([*d.get(p)?, *d.get(p + 1)?]) as f32 / 32768.0;
            out[c].push(s);
        }
    }
    Some(out)
}

fn resample(src: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || src.is_empty() {
        return src.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let out_len = ((src.len() as f64) * ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 / ratio;
        let i0 = pos.floor() as usize;
        let frac = (pos - i0 as f64) as f32;
        let a = src[i0.min(src.len() - 1)];
        let b = src[(i0 + 1).min(src.len() - 1)];
        out.push(a + (b - a) * frac);
    }
    out
}

/// Fit arbitrary PCM16 audio into the target slot (same byte count, header untouched).
pub fn replace_fit(target: &[u8], src: &[u8]) -> Option<Vec<u8>> {
    let tw = info(target)?;
    let sw = info(src)?;
    if !is_pcm16(&tw) || !is_pcm16(&sw) {
        return None;
    }
    let src_ch = decode16(src, &sw)?;

    let mapped: Vec<Vec<f32>> = match (tw.channels, sw.channels) {
        (1, 2) => {
            let a = &src_ch[0];
            let b = &src_ch[1];
            vec![a
                .iter()
                .zip(b.iter())
                .map(|(x, y)| (x + y) * 0.5)
                .collect()]
        }
        (2, 1) => vec![src_ch[0].clone(), src_ch[0].clone()],
        (t, s) if t == s => src_ch,
        _ => return None,
    };

    let tgt_ch = tw.channels as usize;
    let res: Vec<Vec<f32>> = mapped
        .iter()
        .map(|c| resample(c, sw.rate, tw.rate))
        .collect();
    let frames = res.first()?.len().min(tw.data_len / (2 * tgt_ch));

    let mut out = target.to_vec();
    out.get_mut(tw.data_start..tw.data_start + tw.data_len)?
        .fill(0);
    for i in 0..frames {
        for c in 0..tgt_ch {
            let v = (res[c][i] * 32767.0).clamp(-32768.0, 32767.0) as i16;
            let p = tw.data_start + (i * tgt_ch + c) * 2;
            out.get_mut(p..p + 2)?.copy_from_slice(&v.to_le_bytes());
        }
    }
    Some(out)
}

pub fn duration(d: &[u8]) -> Option<f32> {
    let w = info(d)?;
    let frame_bytes = w.channels as f32 * (w.bits as f32 / 8.0);
    if frame_bytes <= 0.0 {
        return None;
    }
    Some(w.data_len as f32 / (w.rate as f32 * frame_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav16(ch: u16, rate: u32, frames: &[Vec<i16>]) -> Vec<u8> {
        let mut data = Vec::new();
        for f in frames {
            for s in f {
                data.extend_from_slice(&s.to_le_bytes());
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&ch.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * ch as u32 * 2).to_le_bytes());
        out.extend_from_slice(&(ch * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    fn wav8(ch: u16, rate: u32, samples: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&ch.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * ch as u32).to_le_bytes());
        out.extend_from_slice(&ch.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        out.extend_from_slice(samples);
        out
    }

    #[test]
    fn silence_8bit_uses_midpoint() {
        let w = wav8(1, 22050, &[10, 200, 128]);
        let s = silence(&w).expect("pcm8 supported");
        let w2 = info(&s).unwrap();
        assert_eq!(&s[w2.data_start..w2.data_start + w2.data_len], &[0x80, 0x80, 0x80]);
    }

    #[test]
    fn gain_8bit_keeps_center() {
        let w = wav8(1, 22050, &[128, 128]);
        let (out, _, _) = gain(&w, 2.0).unwrap();
        assert_eq!(out, w);
    }

    #[test]
    fn stereo_replace_keeps_right_channel() {
        let frames: Vec<Vec<i16>> = (0..100).map(|_| vec![32767, 0]).collect();
        let src = wav16(2, 22050, &frames);
        let tgt = wav16(2, 22050, &vec![vec![0, 0]; 100]);
        let out = replace_fit(&tgt, &src).expect("fits");
        let w = info(&out).unwrap();
        let l = i16::from_le_bytes([out[w.data_start], out[w.data_start + 1]]);
        let r = i16::from_le_bytes([out[w.data_start + 2], out[w.data_start + 3]]);
        assert!(l > 30000, "left {}", l);
        assert!(r.abs() < 100, "right {}", r);
    }

    #[test]
    fn mono_into_stereo_duplicates() {
        let src = wav16(1, 22050, &vec![vec![16384]; 50]);
        let tgt = wav16(2, 22050, &vec![vec![0, 0]; 50]);
        let out = replace_fit(&tgt, &src).expect("fits");
        let w = info(&out).unwrap();
        let l = i16::from_le_bytes([out[w.data_start], out[w.data_start + 1]]);
        let r = i16::from_le_bytes([out[w.data_start + 2], out[w.data_start + 3]]);
        assert_eq!(l, r);
    }

    #[test]
    fn info_rejects_truncated_and_lying_chunks() {
        assert!(info(&[0u8; 8]).is_none());
        let mut w = wav16(1, 22050, &vec![vec![1]; 10]);
        let n = w.len();
        w.truncate(n / 2);
        assert!(info(&w).is_none());
    }

    #[test]
    fn unsupported_formats_are_skipped() {
        let mut w = wav16(1, 22050, &vec![vec![1]; 10]);
        w[20] = 3; // fmt tag -> IEEE float
        assert!(silence(&w).is_none());
        assert!(gain(&w, 2.0).is_none());
    }
}
