//! 系统声音环路回采（Windows WASAPI loopback）。
//!
//! cpal 的 WASAPI 后端对输出（render）设备构建输入流时会自动带上
//! `AUDCLNT_STREAMFLAGS_LOOPBACK`，因此把默认输出设备当作输入源即可拿到
//! 系统正在播放的混音样本，无需虚拟声卡。

use std::sync::mpsc::SyncSender;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};

pub struct Capture {
    /// 刻意持有：stream Drop 时采集停止；字段本身无需读取。
    _stream: Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Capture {
    pub fn describe(&self) -> (String, u32, u16) {
        (self.device_name.clone(), self.sample_rate, self.channels)
    }
}

fn device_name(dev: &Device) -> String {
    dev.description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "(未知)".into())
}

/// 列出可用输出设备，默认输出设备带 `[默认]` 标记。
pub fn list_output_devices() -> Result<()> {
    let host = cpal::default_host();
    let default = host.default_output_device();
    let mut devices: Vec<Device> = host
        .output_devices()
        .context("枚举输出设备失败")?
        .collect();
    devices.sort_by_key(device_name);
    for dev in devices {
        let name = device_name(&dev);
        let is_default = default
            .as_ref()
            .and_then(|d| d.id().ok())
            .zip(dev.id().ok())
            .map(|(a, b)| a == b)
            .unwrap_or(false);
        println!("{}{name}", if is_default { "[默认] " } else { "" });
    }
    Ok(())
}

/// 启动环路回采：把（指定的或系统默认的）输出设备当作输入流打开，
/// 回调里把交织 PCM 样本（f32，-1..1）发送到 channel。
pub fn start_loopback_capture(
    device_match: Option<&str>,
    tx: SyncSender<Vec<f32>>,
) -> Result<Capture> {
    let host = cpal::default_host();
    let device = pick_output_device(&host, device_match)?;

    let dev_name = device_name(&device);
    let supported = device
        .default_output_config()
        .with_context(|| format!("获取设备 \"{dev_name}\" 默认格式失败"))?;
    let sample_rate = supported.sample_rate();
    let channels = supported.channels();
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();

    let stream = build_input_stream(&device, config, sample_format, tx)
        .with_context(|| format!("在输出设备 \"{dev_name}\" 上打开环路回采失败"))?;
    stream.play().context("无法开始采集（设备可能被占用）")?;

    Ok(Capture {
        _stream: stream,
        device_name: dev_name,
        sample_rate,
        channels,
    })
}

fn pick_output_device(host: &cpal::Host, device_match: Option<&str>) -> Result<Device> {
    match device_match {
        None => host
            .default_output_device()
            .context("没有可用的默认输出设备"),
        Some(name) => {
            let needle = name.to_lowercase();
            let matches: Vec<Device> = host
                .output_devices()
                .context("枚举输出设备失败")?
                .filter(|d| device_name(d).to_lowercase().contains(&needle))
                .collect();
            match matches.len() {
                0 => anyhow::bail!("找不到匹配的输出设备: {name}"),
                1 => Ok(matches.into_iter().next().unwrap()),
                n => anyhow::bail!(
                    "设备名 \"{name}\" 匹配到 {n} 个设备，请用 --list-devices 查看后指定更精确的名称"
                ),
            }
        }
    }
}

fn build_input_stream(
    device: &Device,
    config: StreamConfig,
    format: SampleFormat,
    tx: SyncSender<Vec<f32>>,
) -> Result<Stream> {
    let err_fn = |e| eprintln!("采集错误: {e}");
    let stream = match format {
        // WASAPI 共享模式混音格式固定为 float32，这是 Windows 上的主路径。
        SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            config,
            move |samples: &[f32], _| {
                let _ = tx.try_send(samples.to_vec());
            },
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
            config,
            move |samples: &[i16], _| {
                let _ = tx.try_send(
                    samples
                        .iter()
                        .map(|&s| f32::from(s) / 32768.0)
                        .collect(),
                );
            },
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_input_stream::<u16, _, _>(
            config,
            move |samples: &[u16], _| {
                let _ = tx.try_send(
                    samples
                        .iter()
                        .map(|&s| (f32::from(s) / 32768.0) - 1.0)
                        .collect(),
                );
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("不支持的采样格式: {other:?}"),
    };
    Ok(stream)
}
