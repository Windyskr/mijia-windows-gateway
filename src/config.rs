//! 读取 .env：每行 KEY=VALUE，# 开头是注释，值可以用引号括起来。
//! 出错时只返回固定提示、不回显原文，避免把 bindkey 或令牌写进日志。
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

pub struct Device {
    pub address: String,
    pub name: String,
    pub key: [u8; 16],
}
pub struct Config {
    pub root: PathBuf,
    pub devices: Vec<Device>,
    pub interval: i64,
    pub offline_after: i64,
    pub url: String,
    pub token: String,
}

fn parse(text: &str) -> Result<HashMap<String, String>, String> {
    let mut vars = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or(".env 里有无法识别的行（应为 KEY=VALUE）")?;
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        vars.insert(key.trim().to_owned(), value.to_owned());
    }
    Ok(vars)
}

pub fn load(path: &Path, listen: bool) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|_| "无法读取 .env")?;
    let vars = parse(text.trim_start_matches('\u{feff}'))?;
    let get = |key: &str| vars.get(key).map(String::as_str).filter(|v| !v.is_empty());
    let number = |key: &str, fallback: i64| -> Result<i64, String> {
        get(key).map_or(Ok(fallback), |v| {
            v.parse().map_err(|_| format!("{key} 应为整数（秒）"))
        })
    };
    let interval = number("RECORD_INTERVAL", 300)?;
    if interval < 60 {
        return Err("RECORD_INTERVAL 不能小于 60 秒".into());
    }
    let offline_after = number("OFFLINE_AFTER", (3 * interval).max(900))?;
    let url = get("UPLOAD_URL").unwrap_or("").to_owned();
    let token = get("UPLOAD_TOKEN").unwrap_or("").to_owned();
    if !listen && (url.is_empty() || token.is_empty()) {
        return Err("需要在 .env 里填写 UPLOAD_URL 和 UPLOAD_TOKEN（只看解密结果可以用 --listen）".into());
    }
    let mut devices = Vec::new();
    let mut seen = HashSet::new();
    for n in 1.. {
        let Some(mac) = get(format!("DEVICE{n}_MAC").as_str()) else {
            break;
        };
        let address = mac.to_uppercase().replace('-', ":");
        if address.len() != 17
            || address
                .split(':')
                .any(|s| s.len() != 2 || u8::from_str_radix(s, 16).is_err())
        {
            return Err(format!("DEVICE{n}_MAC 格式不正确（应为 A4:C1:38:XX:XX:XX）"));
        }
        if !seen.insert(address.clone()) {
            return Err(format!("DEVICE{n}_MAC 与前面的设备重复"));
        }
        let bad_key = || format!("DEVICE{n}_BINDKEY 应为 32 位十六进制");
        let key_text = get(format!("DEVICE{n}_BINDKEY").as_str()).unwrap_or("");
        if key_text.len() != 32 || !key_text.is_ascii() {
            return Err(bad_key());
        }
        let mut key = [0u8; 16];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&key_text[i * 2..i * 2 + 2], 16).map_err(|_| bad_key())?;
        }
        let name = get(format!("DEVICE{n}_NAME").as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("温湿度计 {}", address.replace(':', "").get(8..).unwrap_or("")));
        devices.push(Device { address, name, key });
    }
    if devices.is_empty() {
        return Err(".env 里至少要配置一台设备（DEVICE1_MAC、DEVICE1_BINDKEY）".into());
    }
    Ok(Config {
        root: path.parent().unwrap_or(Path::new(".")).to_owned(),
        devices,
        interval,
        offline_after,
        url,
        token,
    })
}
