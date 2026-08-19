//! sound-analysis：捕获系统正在播放的声音，生成波形 JSON，POST 到本地 lhm `/inject`。
//!
//! 目标平台 Windows x64，产物为单文件 exe（在打包机上 `cargo build --release`）。

mod audio;
mod inject;
mod waveform;

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;

/// 捕获系统正在播放的声音，生成波形 JSON，POST 到本地 lhm 的 /inject 端点。
#[derive(Parser, Debug)]
#[command(name = "sound-analysis", version, about, long_about = None)]
struct Args {
    /// lhm 服务监听地址
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// lhm 固定端口（默认 18080，与 djgd 一致）
    #[arg(long, default_value_t = 18080)]
    port: u16,

    /// 采集设备名（对输出设备名做不区分大小写的模糊匹配；不指定则用系统默认输出设备环路回采）
    #[arg(long)]
    device: Option<String>,

    /// 单个波形窗口时长（毫秒）
    #[arg(long, default_value_t = 20)]
    window_ms: u64,

    /// 上传间隔（毫秒），必须小于 5000（lhm 注入字段 TTL 为 5 秒）
    #[arg(long, default_value_t = 1000)]
    interval_ms: u64,

    /// 列出可用的输出设备后退出
    #[arg(long)]
    list_devices: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.list_devices {
        audio::list_output_devices()?;
        return Ok(());
    }
    if args.window_ms == 0 {
        bail!("--window-ms 必须大于 0");
    }
    if args.interval_ms == 0 || args.interval_ms >= 5000 {
        bail!("--interval-ms 必须在 1..5000 之间（lhm 注入字段 TTL 为 5 秒）");
    }

    let stop = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler({
        let stop = stop.clone();
        move || stop.store(true, Ordering::SeqCst)
    })
    .context("注册 Ctrl+C 处理器失败")?;
    spawn_parent_stdin_watcher(stop.clone());

    run(&args, stop)
}

/// 与 lhm console 模式一致：父进程（djgd）以管道 stdin 拉起时，父进程关闭 stdin 即退出。
/// 终端（控制台直接运行）或非管道 stdin 下不启用，避免立即误退出。
fn spawn_parent_stdin_watcher(stop: Arc<AtomicBool>) {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return;
    }
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            match std::io::stdin().read(&mut buf) {
                Ok(0) | Err(_) => {
                    stop.store(true, Ordering::SeqCst);
                    return;
                }
                Ok(_) => {}
            }
        }
    });
}

fn run(args: &Args, stop: Arc<AtomicBool>) -> Result<()> {
    let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(64);
    let capture = audio::start_loopback_capture(args.device.as_deref(), tx)?;
    let (device_name, sample_rate, channels) = capture.describe();

    let window_frames = ((sample_rate as u64 * args.window_ms) / 1000).max(1) as usize;
    let batch_bins = (args.interval_ms / args.window_ms).max(1) as usize;
    let mut agg = waveform::Aggregator::new(
        window_frames,
        batch_bins,
        device_name.clone(),
        sample_rate,
        args.window_ms,
    );

    let url = format!("http://{}:{}/inject", args.host, args.port);
    println!(
        "采集设备: {device_name} ({sample_rate} Hz, {channels} ch) | 注入: {url} | 窗口 {}ms，每批 {batch_bins} 窗口",
        args.window_ms
    );

    let agent = inject::build_agent();
    // 注入失败日志节流，避免持续断网时刷屏。
    let mut last_err_log = std::time::Instant::now() - Duration::from_secs(11);

    loop {
        if stop.load(Ordering::SeqCst) {
            println!("收到退出信号，停止采集");
            break;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => {
                if let Some(batch) = agg.push(&chunk, channels) {
                    if let Err(e) = inject::post_batch(&agent, &url, &batch) {
                        if last_err_log.elapsed() >= Duration::from_secs(10) {
                            eprintln!(
                                "[{}] 注入失败: {e:#}",
                                chrono::Local::now().format("%H:%M:%S")
                            );
                            last_err_log = std::time::Instant::now();
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("采集流已断开，退出");
                break;
            }
        }
    }
    Ok(())
}
