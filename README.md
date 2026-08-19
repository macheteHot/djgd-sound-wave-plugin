# djgd-sound-wave-plugin

捕获电脑正在播放的声音，实时生成波形 JSON，POST 到本地 lhm 服务的 `/inject` 端点（固定端口 18080）。产物为 Windows x64 单文件 exe，作为 lhm 的音频波形插件运行。

## 项目目标

1. **采集系统声音**：捕获电脑当前正在播放的音频（环路回采，无需虚拟声卡或麦克风）。
2. **生成波形 JSON**：对音频流按时间窗口计算振幅（min/max/rms），输出 JSON。
3. **注入 lhm**：POST 到本机 lhm 服务固定端口，合并进 SSE 推送，供渲染端实时画波形。

## 技术栈

- **Rust**（edition 2024），目标平台 **Windows x64**，产物为单文件 exe（`target/release/djgd-sound-wave-plugin.exe`）
- 音频采集：[cpal](https://crates.io/crates/cpal)（WASAPI 环路回采）
- CLI：[clap](https://crates.io/crates/clap) derive；HTTP：[ureq](https://crates.io/crates/ureq)（关闭 TLS，仅本地 http）
- 依赖全为纯 Rust，**无 C 编译依赖**，打包机无需额外工具链

## 与 lhm 的集成契约

对接 lhm 服务端（`src/Program.cs`）的插件注入机制：

| 项 | 值 |
| --- | --- |
| 端点 | `POST http://127.0.0.1:18080/inject`（端口与 lhm `DefaultPort` / electron `LHM_FIXED_PORT` 一致，可用 `--port` 覆盖） |
| Body | `{ 字段名: 值, ..., "__fields": { 字段名: 定义 } }` |
| 字段定义 | `label`（非空字符串）与 `unit` 必填，`min`/`max` 可选数值，`group` 透传 |
| 字段有效期 | 不过期，`__fields` 字段定义持久生效 |

### 注入字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `AudioWaveform` | `[{min, max, rms}, ...]` | 每个时间窗口的振幅极值与 RMS，渲染端按此画波形图 |
| `AudioLevel` | number | 整批 RMS 均值，作为当前响度的快捷标量 |

字段会合并进 lhm SSE 推送的 `Standard` 段（同名覆盖），字段定义进入 `FieldDefs` 段供渲染端动态注册。

## 工作原理

```
系统正在播放的声音
        │
        ▼
  环路回采（cpal，输出设备当输入流 → WASAPI 自动 loopback）
        │  交织 PCM（f32，-1..1）
        ▼
  波形聚合（按窗口混音为单声道，算 min/max/rms，凑满一批）
        │
        ▼
  POST /inject（字段 + __fields，失败重试 3 次，错误日志 10s 节流）
```

## 数据格式

单窗口 bin（与项目 README 拟定格式一致）：

```json
{ "min": -0.85, "max": 0.92, "rms": 0.41 }
```

上报 body 示例：

```json
{
  "AudioWaveform": [
    { "min": -0.85, "max": 0.92, "rms": 0.41 },
    { "min": -0.62, "max": 0.71, "rms": 0.28 }
  ],
  "AudioLevel": 0.345,
  "__fields": {
    "AudioWaveform": { "label": "音频波形", "unit": "振幅", "min": -1, "max": 1, "group": "音频" },
    "AudioLevel": { "label": "音频响度", "unit": "RMS", "min": 0, "max": 1, "group": "音频" }
  }
}
```

## 命令行参数

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--host` | `127.0.0.1` | lhm 服务地址 |
| `--port` | `18080` | lhm 固定端口 |
| `--device` | 系统默认输出设备 | 输出设备名模糊匹配（不区分大小写）；不指定则默认设备环路回采 |
| `--window-ms` | `20` | 单个波形窗口时长 |
| `--interval-ms` | `1000` | 上报间隔（毫秒） |
| `--list-devices` | — | 列出可用输出设备后退出 |

## 构建（打包机，Windows x64）

```powershell
cargo build --release
# 产物：target\release\djgd-sound-wave-plugin.exe
```

本机（macOS）只能做类型检查与测试（`check` 不链接，无需 MSVC）：

```bash
cargo check --target x86_64-pc-windows-msvc   # 类型检查
cargo test                                    # 单元测试（波形聚合 / 注入契约）
```

## 运行

```powershell
djgd-sound-wave-plugin.exe                 # 默认：采集默认输出设备，注入 127.0.0.1:18080
djgd-sound-wave-plugin.exe --list-devices  # 查看设备名，配合 --device 指定
```

- 由 djgd 以**管道 stdin** 拉起时：父进程关闭 stdin 即退出（与 lhm console 模式生命周期一致）。
- 控制台直接运行：Ctrl+C 退出。

## 开发计划

- [x] 选定技术栈（Rust + cpal + ureq，Windows x64 exe）
- [x] 实现系统声音环路回采（WASAPI loopback，无需虚拟声卡）
- [x] 实现波形聚合并输出 JSON（窗口 min/max/rms）
- [x] 实现 /inject 注入（重试、字段定义）
- [x] 端到端联调（对真实 lhm 服务验证 SSE 合并效果）

## 许可证

[MIT](LICENSE)
