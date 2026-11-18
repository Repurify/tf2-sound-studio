#[derive(Clone, Copy, Debug)]
pub struct WavInfo {
    pub fmt: u16,
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
    pub data_start: usize,
    pub data_len: usize,
}

fn u16at(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}
fn u32at(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

pub fn info(d: &[u8]) -> Option<WavInfo> {
    if d.len() < 44 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return None;
    }
    let mut p = 12usize;
    let mut fmt = None;
    let mut data = None;
    while p + 8 <= d.len() {
        let id = &d[p..p + 4];
        let sz = u32at(d, p + 4) as usize;
        let body = p + 8;
        if id == b"fmt " && body + 16 <= d.len() {
            fmt = Some((
                u16at(d, body),
                u16at(d, body + 2),
                u32at(d, body + 4),
                u16at(d, body + 14),
            ));
        } else if id == b"data" {
            data = Some((body, sz.min(d.len().saturating_sub(body))));
            break;
        }
        p = body + sz + (sz & 1);
    }
    let (fmt_tag, channels, rate, bits) = fmt?;
    let (data_start, data_len) = data?;
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
    w.fmt == 1 && w.bits == 16 && w.channels >= 1 && w.channels <= 2
}

/// Keep RIFF header, replace audio payload with zeros (same total length).
pub fn silence(d: &[u8]) -> Vec<u8> {
    match info(d) {
        Some(w) => {
            let mut out = d.to_vec();
            for b in out[w.data_start..w.data_start + w.data_len].iter_mut() {
                *b = 0;
            }
            out
        }
        None => d.to_vec(),
    }
}

pub fn gain(d: &[u8], factor: f32) -> (Vec<u8>, usize, usize) {
    let mut out = d.to_vec();
    let w = match info(d) {
        Some(w) if is_pcm16(&w) => w,
        _ => return (out, 0, 0),
    };
    let n = w.data_len / 2;
    let mut clipped = 0usize;
    for i in 0..n {
        let p = w.data_start + i * 2;
        let s = i16::from_le_bytes([out[p], out[p + 1]]);
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
        out[p..p + 2].copy_from_slice(&v.to_le_bytes());
    }
    (out, clipped, n)
}

fn channels(d: &[u8], w: &WavInfo) -> Vec<Vec<f32>> {
    let n = w.data_len / 2;
    let ch = w.channels as usize;
    let mut out = vec![Vec::with_capacity(n / ch + 1); ch];
    for i in 0..n {
        let p = w.data_start + i * 2;
        let s = i16::from_le_bytes([d[p], d[p + 1]]) as f32 / 32768.0;
        out[i % ch].push(s);
    }
    out
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

/// Convert arbitrary PCM16 audio so it fits the target slot exactly.
pub fn replace_fit(target: &[u8], src: &[u8]) -> Option<Vec<u8>> {
    let tw = info(target)?;
    let sw = info(src)?;
    if !is_pcm16(&tw) || !is_pcm16(&sw) {
        return None;
    }
    let sch = channels(src, &sw);
    let mixed: Vec<f32> = if tw.channels == 1 && sw.channels == 2 {
        let (a, b) = (&sch[0], &sch[1]);
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| (x + y) * 0.5)
            .collect()
    } else if tw.channels == 2 && sw.channels == 1 {
        sch[0].clone()
    } else {
        sch[0].clone()
    };
    let res = resample(&mixed, sw.rate, tw.rate);

    let slot_frames = tw.data_len / (tw.channels as usize * 2);
    let mut out = target.to_vec();
    out[tw.data_start..tw.data_start + tw.data_len].fill(0);
    let frames = res.len().min(slot_frames);
    for i in 0..frames {
        let v = (res[i] * 32767.0).clamp(-32768.0, 32767.0) as i16;
        let bytes = v.to_le_bytes();
        for c in 0..tw.channels as usize {
            let p = tw.data_start + (i * tw.channels as usize + c) * 2;
            out[p..p + 2].copy_from_slice(&bytes);
        }
    }
    Some(out)
}

pub fn duration(d: &[u8]) -> Option<f32> {
    let w = info(d)?;
    Some(w.data_len as f32 / (w.rate as f32 * w.channels as f32 * (w.bits as f32 / 8.0)))
}
