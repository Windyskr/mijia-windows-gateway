use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::SyncSender,
};
use windows::{
    Devices::{
        Bluetooth::Advertisement::*,
        Radios::{Radio, RadioKind, RadioState},
    },
    Foundation::TypedEventHandler,
    Storage::Streams::{DataReader, DataWriter},
    core::Result,
};

pub struct Beacon {
    pub address: String,
    pub data: Vec<u8>,
}
pub struct Listener {
    watcher: BluetoothLEAdvertisementWatcher,
    token: i64,
}
impl Listener {
    pub fn start(tx: SyncSender<Beacon>, last_adv: Arc<AtomicU64>) -> Result<Self> {
        let writer = DataWriter::new()?;
        writer.WriteBytes(&[0x95, 0xfe])?;
        let pattern = BluetoothLEAdvertisementBytePattern::new()?;
        pattern.SetDataType(0x16)?;
        pattern.SetOffset(0)?;
        pattern.SetData(&writer.DetachBuffer()?)?;
        let filter = BluetoothLEAdvertisementFilter::new()?;
        filter.BytePatterns()?.Append(&pattern)?;
        let watcher = BluetoothLEAdvertisementWatcher::new()?;
        watcher.SetAdvertisementFilter(&filter)?;
        watcher.SetScanningMode(BluetoothLEScanningMode::Passive)?;
        let token = watcher.Received(&TypedEventHandler::new(
            move |_, args: windows::core::Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    last_adv.store(crate::now() as u64, Ordering::Relaxed);
                    let number = args.BluetoothAddress()?;
                    let address = (0..6)
                        .rev()
                        .map(|n| format!("{:02X}", (number >> (n * 8)) & 255))
                        .collect::<Vec<_>>()
                        .join(":");
                    for section in args.Advertisement()?.DataSections()? {
                        if section.DataType()? != 0x16 {
                            continue;
                        }
                        let buffer = section.Data()?;
                        let reader = DataReader::FromBuffer(&buffer)?;
                        let mut bytes = vec![0; buffer.Length()? as usize];
                        reader.ReadBytes(&mut bytes)?;
                        if bytes.starts_with(&[0x95, 0xfe]) {
                            let _ = tx.try_send(Beacon {
                                address: address.clone(),
                                data: bytes[2..].to_vec(),
                            });
                        }
                    }
                }
                Ok(())
            },
        ))?;
        watcher.Start()?;
        Ok(Self { watcher, token })
    }
    pub fn running(&self) -> bool {
        self.watcher
            .Status()
            .is_ok_and(|s| s == BluetoothLEAdvertisementWatcherStatus::Started)
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.watcher.RemoveReceived(self.token);
        let _ = self.watcher.Stop();
    }
}
pub fn radio_error() -> Option<String> {
    let radios = Radio::GetRadiosAsync().ok()?.join().ok()?;
    for r in radios {
        if r.Kind().ok()? == RadioKind::Bluetooth {
            return match r.State().ok()? {
                RadioState::Off => Some("Windows 蓝牙已关闭，请在系统设置中打开蓝牙".into()),
                RadioState::Disabled => Some("蓝牙适配器已被禁用".into()),
                _ => None,
            };
        }
    }
    Some("没有找到蓝牙适配器".into())
}
