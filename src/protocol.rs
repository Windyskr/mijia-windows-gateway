use aes::Aes128;
use ccm::{
    Ccm,
    aead::{
        Aead, KeyInit, Payload,
        consts::{U4, U12},
    },
};

#[derive(Default, Debug)]
pub struct Values {
    pub temperature: Option<f64>,
    pub humidity: Option<i64>,
    pub battery: Option<i64>,
}
// nonce: 6 字节逆序 MAC + 2 字节产品编号 + 帧计数 + 3 字节扩展计数。
pub fn decrypt(data: &[u8], address: &str, key: &[u8; 16]) -> Result<Option<Vec<u8>>, ()> {
    if data.len() < 5 {
        return Ok(None);
    }
    let fc = u16::from_le_bytes([data[0], data[1]]);
    if fc >> 12 < 4 || fc & 0x0008 == 0 || fc & 0x0040 == 0 {
        return Ok(None);
    }
    let mut nonce = [0u8; 12];
    let mut i = 5;
    if fc & 0x0010 != 0 {
        if data.len() < 11 {
            return Ok(None);
        }
        nonce[..6].copy_from_slice(&data[5..11]);
        i = 11;
    } else {
        for (n, part) in address.split(':').rev().enumerate() {
            nonce[n] = u8::from_str_radix(part, 16).map_err(|_| ())?;
        }
    }
    if fc & 0x0020 != 0 {
        let Some(cap) = data.get(i) else {
            return Ok(None);
        };
        i += if cap & 0x20 != 0 { 3 } else { 1 };
    }
    if data.len() < i + 8 {
        return Ok(None);
    }
    nonce[6..9].copy_from_slice(&data[2..5]);
    nonce[9..].copy_from_slice(&data[data.len() - 7..data.len() - 4]);
    let mut ciphertext = data[i..data.len() - 7].to_vec();
    ciphertext.extend_from_slice(&data[data.len() - 4..]);
    Ccm::<Aes128, U4, U12>::new(key.into())
        .decrypt(
            (&nonce).into(),
            Payload {
                msg: &ciphertext,
                aad: &[0x11],
            },
        )
        .map(Some)
        .map_err(|_| ())
}
pub fn objects(payload: &[u8]) -> Values {
    let mut values = Values::default();
    let mut pos = 0;
    while pos + 3 <= payload.len() {
        let oid = u16::from_le_bytes([payload[pos], payload[pos + 1]]);
        let n = payload[pos + 2] as usize;
        let Some(v) = payload.get(pos + 3..pos + 3 + n) else {
            break;
        };
        match (oid, n) {
            (0x4801 | 0x4c01, 4) => {
                let t = f32::from_le_bytes(v.try_into().unwrap()) as f64;
                if t.is_finite() {
                    values.temperature = Some((t * 100.0).round_ties_even() / 100.0);
                }
            }
            (0x4802 | 0x4c02, 1) => values.humidity = Some(v[0] as i64),
            (0x4803 | 0x100a, 1) => values.battery = Some(v[0] as i64),
            (0x1004, 2) => {
                values.temperature = Some(i16::from_le_bytes(v.try_into().unwrap()) as f64 / 10.0)
            }
            (0x1006, 2) => {
                values.humidity = Some((u16::from_le_bytes(v.try_into().unwrap()) / 10) as i64)
            }
            (0x100d, 4) => {
                values.temperature = Some(i16::from_le_bytes([v[0], v[1]]) as f64 / 10.0);
                values.humidity = Some((u16::from_le_bytes([v[2], v[3]]) / 10) as i64);
            }
            _ => (),
        }
        pos += 3 + n;
    }
    values
}
