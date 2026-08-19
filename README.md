# SoundAnalysis

捕获电脑正在播放的声音，实时分析并生成波形图的 JSON 数据，然后 POST 到指定地址。

## 项目目标

1. **采集系统声音**：捕获电脑当前正在播放的音频（不限于麦克风输入）。
2. **生成波形 JSON**：对音频流按时间窗口采样，计算振幅/音量数据，输出为 JSON 格式。
3. **上传到指定地址**：通过 HTTP POST 将波形数据发送到可配置的目标地址，供下游服务（如可视化大屏、Web 前端）消费。

## 工作原理

```
系统正在播放的音频
        │
        ▼
  声音采集模块 ──► 音频数据（PCM）
        │
        ▼
  波形分析模块 ──► 按窗口计算振幅 → 波形 JSON
        │
        ▼
  HTTP 上传模块 ──► POST → 指定地址
```

## 声音采集方案（按操作系统）

| 平台 | 方案 |
| --- | --- |
| macOS | 虚拟声卡（如 [BlackHole](https://github.com/ExistentialAudio/BlackHole)）将系统输出重定向到可录制设备，或用 `ffmpeg -f avfoundation` 采集 |
| Windows | WASAPI 环路回采（loopback），可用 ffmpeg / Python `soundcard` 库 |
| Linux | PulseAudio / PipeWire 的 monitor 源（如 `pulse.monitor`） |

> 采集方案依赖最终选定的开发语言/框架，实现前请先确认目标平台。

## 波形 JSON 数据格式（拟定）

```json
{
  "device": "BlackHole 2ch",
  "sample_rate": 44100,
  "timestamp": "2026-08-19T10:00:00+08:00",
  "duration_ms": 1000,
  "bins": [
    { "min": -0.85, "max": 0.92, "rms": 0.41 },
    { "min": -0.62, "max": 0.71, "rms": 0.28 }
  ]
}
```

字段说明：

- `bins`：每个时间窗口（如 20ms）的波形数据，包含该窗口内的最小/最大振幅和均方根（RMS）。
- 各窗口的 `min` / `max` 可直接用于绘制波形图，`rms` 反映响度。
- 具体字段名和粒度在实现时可根据下游需求调整。

## 配置

| 配置项 | 说明 | 默认值 |
| --- | --- | --- |
| `CAPTURE_DEVICE` | 采集设备名称 | 系统默认输出设备 |
| `WINDOW_MS` | 单个波形窗口时长 | 20 |
| `INTERVAL_MS` | 上传间隔（一批窗口） | 1000 |
| `POST_URL` | 波形数据上传地址 | 必填 |
| `AUTH_TOKEN` | 可选的上传鉴权 token | 空 |

配置通过环境变量或配置文件读取（实现时确定）。

## 快速开始

> 待实现后补充安装与运行步骤。

## 开发计划

- [ ] 选定技术栈（语言、采集库、HTTP 客户端）
- [ ] 实现系统声音采集模块
- [ ] 实现波形分析并输出 JSON
- [ ] 实现 HTTP POST 上传（含重试、鉴权）
- [ ] 端到端联调与测试

## 许可证

待定
