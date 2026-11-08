# mijia-windows-gateway

Use a Windows PC as the hub for Xiaomi Mijia BLE thermometers: it passively receives and decrypts stock-firmware MiBeacon v5 advertisements (no flashing, no connection to the device, no Home Assistant) and uploads the readings to your own server.

用 Windows 电脑代替米家中枢网关：被动接收米家温湿度计（原厂固件）的加密蓝牙广播，在本地解密后上传到你自己的服务器。

常见的不用中枢的方案，要么需要一台运行 Home Assistant 的主机，要么要另买 ESP32 或树莓派当网关，或者要给温湿度计刷第三方固件。这个程序只需要一台开着的 Windows 电脑：

- 原厂固件，不用刷机，米家 App 照常使用。
- 不连接设备，只接收设备本来就在发的广播，不会额外耗电；数据不经过米家云。
- 单个 exe，不需要 Home Assistant、MQTT 或 Python，常驻内存约 5 MB。
- 网络或服务器中断时，读数先存在本地（用 Windows 自带的 winsqlite3.dll），恢复后自动补传。
- 无窗口运行，同时只允许一个实例；可以开机自启（当前用户的“启动”文件夹，不需要管理员权限）。

## 支持的设备

已在原厂固件上实测：

- 米家温湿度计 3（MJWSD05MMC）
- 米家蓝牙温湿度计 2（LYWSD03MMC）

其他使用 MiBeacon v5 加密广播、对象编号相同的米家温湿度计理论上也能用。

## 准备

1. Windows 10/11，电脑有蓝牙。
2. 每台设备的 MAC 地址和 bindkey（32 位十六进制）。bindkey 是米家 App 绑定设备时生成的，可以用开源工具（例如 Xiaomi Cloud Tokens Extractor）登录小米账号获取；在米家 App 里重新绑定后会改变。
3. 一个接收上传的 HTTP 接口，格式见下文“上传格式”。只想看解密结果的话可以先不准备。

## 配置

复制 `.env.example` 为 `.env` 并填写（`.env` 已在 `.gitignore` 里）。程序从 exe 所在目录逐级往上找 `.env`，也可以用 `--config <路径>` 指定；`logs\` 和 `data\` 会放在 `.env` 所在的目录。

## 编译

有 Visual Studio 生成工具（MSVC）时：

```
cargo build --release
```

没有的话，可以用 [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) 编译，不需要管理员权限：

```
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
rustup target add x86_64-pc-windows-gnullvm --toolchain stable-x86_64-pc-windows-gnu
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -LlvmMingw <llvm-mingw 解压目录>
```

两种方式的产物都是 `target\release\mijia-windows-gateway.exe`。

## 运行

| 命令 | 作用 |
|---|---|
| `mijia-windows-gateway.exe --listen` | 只监听并记录解密结果，不保存读数、不上传，用来检查 MAC 和 bindkey |
| `mijia-windows-gateway.exe --console` | 正常运行，日志同时打印到当前控制台 |
| `mijia-windows-gateway.exe` | 正常运行，无窗口 |

日志在 `logs\agent.log`（`--listen` 时是 `logs\listen.log`）。

开机自启：

```
powershell -ExecutionPolicy Bypass -File scripts\install-autostart.ps1     # 设置并立即启动
powershell -ExecutionPolicy Bypass -File scripts\uninstall-autostart.ps1   # 取消并停止
```

## 记录规则

- 每条广播只带一个值：温度、湿度或电量（LYWSD03MMC 偶尔会把温度和湿度放在同一条里）。程序记住每台设备最新的温度和湿度。
- 温度和湿度都收到过以后，每台设备每 `RECORD_INTERVAL` 秒最多记一条：内容是当时最新的温度和湿度，时间是最近一条广播的时间。
- 电量只用广播里收到的值。
- 15 分钟收不到任何广播会自动重启监听；蓝牙关闭时会在状态里报告错误，程序不会自己去打开蓝牙。

## 上传格式

每 60 秒向 `UPLOAD_URL` 发一次（没有新读数也会发，可以当心跳用）：

```
POST <UPLOAD_URL>
Authorization: Bearer <UPLOAD_TOKEN>
Content-Type: application/json

{
  "snapshot": {
    "now": 1762570000.5, "started": 1762560000.1, "interval": 300, "offline_after": 900,
    "bluetooth_error": null,
    "devices": [
      {"address": "A4:C1:38:XX:XX:XX", "name": "室内", "mode": "broadcast", "online": true,
       "temperature": 23.4, "humidity": 42, "battery": 100, "battery_ts": 1762500000.0,
       "last_ok": 1762569990.2, "failures": 0, "last_error": null}
    ]
  },
  "readings": [
    {"id": 1, "address": "A4:C1:38:XX:XX:XX", "ts": 1762569990,
     "temperature": 23.4, "humidity": 42, "voltage": null, "battery": null}
  ]
}
```

- `snapshot.devices` 是每台设备的当前状态（还有几个与连接读取相关的字段，被动监听时不用）。
- `readings` 是还没上传成功的读数，每次最多 1000 条；服务器返回 2xx 后本地才删除。失败重试时可能重复提交，接收端按 `(address, ts)` 去重即可。
- 时间都是 Unix 时间戳（秒）。

## 许可

MIT
