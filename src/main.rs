#![windows_subsystem = "windows"]
mod bluetooth;
mod config;
mod protocol;
mod store;
mod upload;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::{
            Console::{ATTACH_PARENT_PROCESS, AttachConsole},
            Threading::CreateMutexW,
            WinRT::{RO_INIT_MULTITHREADED, RoInitialize},
        },
    },
    core::w,
};

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
struct Logger {
    path: PathBuf,
    console: bool,
}
static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();
fn note(message: impl AsRef<str>) {
    if let Some(logger) = LOGGER.get() {
        if let Ok(logger) = logger.lock() {
            if fs::metadata(&logger.path).is_ok_and(|m| m.len() >= 1_000_000) {
                let _ = fs::remove_file(logger.path.with_extension("log.3"));
                for n in (1..3).rev() {
                    let _ = fs::rename(
                        logger.path.with_extension(format!("log.{n}")),
                        logger.path.with_extension(format!("log.{}", n + 1)),
                    );
                }
                let _ = fs::rename(&logger.path, logger.path.with_extension("log.1"));
            }
            let line = format!("{:.3} {}\n", now(), message.as_ref());
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&logger.path)
            {
                let _ = file.write_all(line.as_bytes());
            }
            if logger.console {
                let _ = std::io::stdout().write_all(line.as_bytes());
            }
        }
    }
}
struct Instance(HANDLE);
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
fn instance(listen: bool) -> Result<Option<Instance>, String> {
    unsafe {
        let handle = CreateMutexW(
            None,
            false,
            if listen {
                w!("Local\\MijiaWindowsGatewayListen")
            } else {
                w!("Local\\MijiaWindowsGateway")
            },
        )
        .map_err(|_| "无法创建实例互斥锁")?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return Ok(None);
        }
        Ok(Some(Instance(handle)))
    }
}
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct DeviceState {
    address: String,
    name: String,
    model: Option<String>,
    firmware: Option<String>,
    last_ok: Option<f64>,
    temperature: Option<f64>,
    humidity: Option<i64>,
    voltage: Option<f64>,
    battery: Option<i64>,
    battery_ts: Option<f64>,
    last_attempt: Option<f64>,
    last_error: Option<String>,
    failures: u64,
    next_poll: f64,
    polling: bool,
    connected_seconds: Option<f64>,
    mode: String,
    #[serde(skip)]
    last_frame: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_saved: Option<f64>,
}
impl DeviceState {
    fn save(&mut self, db: &store::Store, interval: i64) -> Result<(), String> {
        let mut reading = None;
        let mut saved = self.last_saved;
        if let (Some(t), Some(h), Some(ok)) = (self.temperature, self.humidity, self.last_ok) {
            if self
                .last_saved
                .is_none_or(|s| ok > s && now() - s >= interval as f64)
            {
                let battery = if self
                    .battery_ts
                    .is_some_and(|ts| self.last_saved.is_none_or(|s| ts > s))
                {
                    self.battery
                } else {
                    None
                };
                reading =
                    Some(json!({"ts":ok as i64,"temperature":t,"humidity":h,"battery":battery}));
                saved = Some(ok);
            }
        }
        let mut state = serde_json::to_value(&*self).map_err(|_| "无法序列化设备状态")?;
        state["last_saved"] = json!(saved);
        db.persist(&self.address, &state, reading.as_ref())?;
        self.last_saved = saved;
        if let Some(r) = reading {
            let battery = r["battery"]
                .as_i64()
                .map(|b| format!("，电量 {b}%"))
                .unwrap_or_default();
            note(format!(
                "{}：{} °C，{} %RH{battery}（广播记录）",
                self.name, r["temperature"], r["humidity"]
            ));
        }
        Ok(())
    }
}
fn snapshot(
    cfg: &config::Config,
    states: &[DeviceState],
    started: f64,
    error: &Option<String>,
) -> Value {
    let current = now();
    let devices: Vec<Value> = states
        .iter()
        .map(|s| {
            let mut v = serde_json::to_value(s).unwrap();
            v.as_object_mut().unwrap().remove("last_saved");
            v["online"] = json!(
                s.last_ok
                    .is_some_and(|t| current - t <= cfg.offline_after as f64)
            );
            v
        })
        .collect();
    json!({"now":current,"started":started,"interval":cfg.interval,"offline_after":cfg.offline_after,"bluetooth_error":error,"devices":devices})
}
fn default_config() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        for parent in exe.ancestors().skip(1) {
            if parent.join(".env").is_file() {
                return parent.join(".env");
            }
        }
    }
    PathBuf::from(".env")
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let listen = args.iter().any(|s| s == "--listen");
    let console = listen || args.iter().any(|s| s == "--console");
    if console {
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
    let mut path = default_config();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" | "-c" => {
                i += 1;
                path = PathBuf::from(args.get(i).ok_or("--config 缺少文件路径")?);
            }
            "--listen" | "--console" => (),
            _ => return Err("参数无效：支持 --config <.env 路径>、--listen、--console".into()),
        }
        i += 1;
    }
    let path = fs::canonicalize(path).map_err(|_| "找不到 .env，请复制 .env.example 后填写")?;
    let root = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(root.join("logs")).map_err(|_| "无法创建日志目录")?;
    let _ = LOGGER.set(Mutex::new(Logger {
        path: root.join(if listen {
            "logs/listen.log"
        } else {
            "logs/agent.log"
        }),
        console,
    }));
    let cfg = config::load(&path, listen)?;
    let Some(_instance) = instance(listen)? else {
        note("已有相同模式的程序在运行，本次启动退出");
        return Ok(());
    };
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED).map_err(|_| "无法初始化 Windows Runtime")?;
    }
    let db = if listen {
        None
    } else {
        Some(store::Store::open(&cfg.root.join("data/outbox.db"))?)
    };
    let mut states = Vec::new();
    for d in &cfg.devices {
        let mut st: DeviceState = if let Some(db) = &db {
            db.restore(&d.address)?
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default()
        } else {
            DeviceState::default()
        };
        st.address = d.address.clone();
        st.name = d.name.clone();
        st.mode = "broadcast".into();
        st.last_error = None;
        st.failures = 0;
        st.voltage = None;
        states.push(st);
    }
    let started = now();
    note(format!(
        "启动{}，设备 {} 台，记录间隔 {} 秒，进程 {}",
        if listen {
            "只监听模式（不写入数据库，不上传）"
        } else {
            "采集上传"
        },
        states.len(),
        cfg.interval,
        std::process::id()
    ));
    let (adv_tx, adv_rx) = mpsc::sync_channel(256);
    let last_adv = Arc::new(AtomicU64::new(now() as u64));
    let (job_tx, job_rx) = mpsc::sync_channel::<(Vec<u8>, i64, usize)>(1);
    let (result_tx, result_rx) = mpsc::channel();
    if !listen {
        let url = cfg.url.clone();
        let token = cfg.token.clone();
        std::thread::spawn(move || {
            while let Ok((body, id, count)) = job_rx.recv() {
                let result = upload::post(&url, &token, &body);
                if result_tx.send((result, id, count)).is_err() {
                    break;
                }
            }
        });
    }
    let mut listener: Option<bluetooth::Listener> = None;
    let mut bluetooth_error = None;
    let mut radio_available = true;
    let mut next_scan = 0.0;
    let mut next_check = 0.0;
    let mut next_upload = 0.0;
    let mut uploading = false;
    let mut failing_since = None;
    let mut previous_tick = now();
    loop {
        let current = now();
        if current - previous_tick > 90.0 || current < previous_tick - 5.0 {
            listener = None;
            next_scan = 0.0;
            next_upload = 0.0;
            note("检测到系统唤醒或时钟变化，重新启动监听");
        }
        previous_tick = current;
        if current >= next_check {
            next_check = current + 30.0;
            let radio = bluetooth::radio_error();
            radio_available = radio.is_none();
            if radio.is_some() {
                listener = None;
            }
            if radio != bluetooth_error && radio.is_some() {
                note(radio.as_ref().unwrap());
            }
            if radio.is_some() {
                bluetooth_error = radio;
            }
            if let Some(db) = &db {
                for st in &mut states {
                    if let Err(e) = st.save(db, cfg.interval) {
                        note(e);
                    }
                }
            }
        }
        if listener.as_ref().is_some_and(|l| !l.running()) {
            let message = bluetooth::radio_error()
                .unwrap_or_else(|| "蓝牙扫描出错：监听已停止，30 秒后重试".into());
            note(&message);
            bluetooth_error = Some(message);
            listener = None;
            next_scan = current + 30.0;
        }
        if listener.is_some() && current - last_adv.load(Ordering::Relaxed) as f64 >= 900.0 {
            note("15 分钟未收到小米广播，重新启动监听");
            listener = None;
            next_scan = 0.0;
        }
        if listener.is_none() && radio_available && current >= next_scan {
            next_scan = current + 30.0;
            match bluetooth::Listener::start(adv_tx.clone(), last_adv.clone()) {
                Ok(l) => {
                    listener = Some(l);
                    last_adv.store(current as u64, Ordering::Relaxed);
                    bluetooth_error = None;
                    note("开始被动监听广播（不连接设备）");
                }
                Err(e) => {
                    let message = bluetooth::radio_error()
                        .unwrap_or_else(|| format!("蓝牙扫描出错：系统代码 {}", e.code().0));
                    if bluetooth_error.as_ref() != Some(&message) {
                        note(format!("监听广播失败：{message}；30 秒后重试"));
                    }
                    bluetooth_error = Some(message);
                }
            }
        }
        while let Ok(beacon) = adv_rx.try_recv() {
            let Some(index) = states.iter().position(|s| s.address == beacon.address) else {
                continue;
            };
            let st = &mut states[index];
            if beacon
                .data
                .get(4)
                .is_some_and(|f| Some(*f) == st.last_frame)
            {
                continue;
            }
            match protocol::decrypt(&beacon.data, &st.address, &cfg.devices[index].key) {
                Ok(Some(payload)) => {
                    st.last_frame = beacon.data.get(4).copied();
                    let values = protocol::objects(&payload);
                    if values.temperature.is_none()
                        && values.humidity.is_none()
                        && values.battery.is_none()
                    {
                        continue;
                    }
                    st.last_error = None;
                    st.failures = 0;
                    st.last_ok = Some(now());
                    if let Some(t) = values.temperature {
                        st.temperature = Some(t);
                    }
                    if let Some(h) = values.humidity {
                        st.humidity = Some(h);
                    }
                    if let Some(b) = values.battery {
                        st.battery = Some(b);
                        st.battery_ts = st.last_ok;
                    }
                    if listen {
                        let mut fields = Vec::new();
                        if let Some(t) = values.temperature {
                            fields.push(format!("温度 {t:.2} °C"));
                        }
                        if let Some(h) = values.humidity {
                            fields.push(format!("湿度 {h} %RH"));
                        }
                        if let Some(b) = values.battery {
                            fields.push(format!("电量 {b}%"));
                        }
                        note(format!(
                            "{} 解码成功：{}，帧计数 {}",
                            st.name,
                            fields.join("，"),
                            beacon.data[4]
                        ));
                    }
                    if let Some(db) = &db {
                        if let Err(e) = st.save(db, cfg.interval) {
                            note(e);
                        }
                    }
                }
                Ok(None) => (),
                Err(()) => {
                    if st.last_error.is_none() {
                        note(format!("{} 广播解密失败，bindkey 不正确", st.name));
                    }
                    st.failures += 1;
                    st.last_error =
                        Some("解密失败：bindkey 不正确（设备在米家里重新绑定后密钥会变）".into());
                }
            }
        }
        if let Ok((result, id, count)) = result_rx.try_recv() {
            uploading = false;
            next_upload = now() + 60.0;
            match result {
                Ok(()) => {
                    let deleted = if id > 0 {
                        db.as_ref().unwrap().delete_upto(id)
                    } else {
                        Ok(())
                    };
                    if let Err(e) = deleted {
                        note(e);
                    } else {
                        if count > 0 {
                            note(format!("已上传 {count} 条读数"));
                        }
                        if count == 1000 {
                            next_upload = 0.0;
                        }
                    }
                    if let Some(since) = failing_since.take() {
                        note(format!("上传恢复（中断了 {:.0} 秒）", now() - since));
                    }
                }
                Err(e) => {
                    if failing_since.is_none() {
                        failing_since = Some(now());
                        note(format!("上传失败：{e}（读数保留在本地队列，稍后重试）"));
                    }
                }
            }
        }
        if !listen && !uploading && current >= next_upload {
            match db.as_ref().unwrap().pending() {
                Ok(rows) => {
                    let id = rows.last().and_then(|r| r["id"].as_i64()).unwrap_or(0);
                    let count = rows.len();
                    let payload = json!({"snapshot":snapshot(&cfg,&states,started,&bluetooth_error),"readings":rows});
                    let body = serde_json::to_vec(&payload).map_err(|_| "无法序列化上传内容")?;
                    job_tx
                        .send((body, id, count))
                        .map_err(|_| "上传线程意外结束")?;
                    uploading = true;
                }
                Err(e) => {
                    note(e);
                    next_upload = current + 60.0;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
fn main() {
    if let Err(e) = run() {
        note(format!("程序启动或运行失败：{e}"));
        std::process::exit(1);
    }
}
