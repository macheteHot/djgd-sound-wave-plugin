//! 波形窗口聚合：把 PCM 样本按窗口计算 min/max/rms，凑满一批输出。

use serde::Serialize;

/// 单个时间窗口的波形数据（与项目 README 的 bin 格式一致）。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Bin {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

/// 一批窗口的波形，对应一次 /inject 上报。
#[derive(Debug, Clone, Serialize)]
pub struct WaveformBatch {
    pub device: String,
    pub sample_rate: u32,
    pub window_ms: u64,
    pub duration_ms: u64,
    pub bins: Vec<Bin>,
}

pub struct Aggregator {
    window_frames: usize,
    batch_bins: usize,
    device: String,
    sample_rate: u32,
    window_ms: u64,
    /// 当前窗口累积的单声道样本。
    pending: Vec<f32>,
    min: f32,
    max: f32,
    sum_sq: f64,
    bins: Vec<Bin>,
}

impl Aggregator {
    pub fn new(
        window_frames: usize,
        batch_bins: usize,
        device: String,
        sample_rate: u32,
        window_ms: u64,
    ) -> Self {
        Self {
            window_frames,
            batch_bins,
            device,
            sample_rate,
            window_ms,
            pending: Vec::with_capacity(window_frames),
            min: 0.0,
            max: 0.0,
            sum_sq: 0.0,
            bins: Vec::with_capacity(batch_bins),
        }
    }

    /// 推送交织采样 PCM（channels 声道），返回凑满的一批波形（未满则 None）。
    /// 多声道按帧平均混为单声道；窗口边界跨越多次 push 时样本会正确累积。
    pub fn push(&mut self, interleaved: &[f32], channels: u16) -> Option<WaveformBatch> {
        let ch = usize::from(channels.max(1));
        for frame in interleaved.chunks_exact(ch) {
            let mono: f32 = frame.iter().sum::<f32>() / ch as f32;
            self.accumulate(mono);
        }
        (self.bins.len() >= self.batch_bins).then(|| self.take_batch())
    }

    fn accumulate(&mut self, sample: f32) {
        if self.pending.is_empty() {
            self.min = sample;
            self.max = sample;
            self.sum_sq = 0.0;
        }
        self.min = self.min.min(sample);
        self.max = self.max.max(sample);
        self.sum_sq += f64::from(sample) * f64::from(sample);
        self.pending.push(sample);
        if self.pending.len() >= self.window_frames {
            let n = self.pending.len() as f64;
            self.bins.push(Bin {
                min: self.min,
                max: self.max,
                rms: (self.sum_sq / n).sqrt() as f32,
            });
            self.pending.clear();
        }
    }

    fn take_batch(&mut self) -> WaveformBatch {
        let bins = std::mem::take(&mut self.bins);
        WaveformBatch {
            device: self.device.clone(),
            sample_rate: self.sample_rate,
            window_ms: self.window_ms,
            duration_ms: self.window_ms * bins.len() as u64,
            bins,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agg() -> Aggregator {
        Aggregator::new(4, 2, "Test".into(), 44100, 10)
    }

    #[test]
    fn bin_math_min_max_rms() {
        // 窗口 4 帧：[1.0, -0.5, 0.0, 0.5]
        // min=-0.5, max=1.0, rms=sqrt(1.5/4)=0.6124
        let mut a = agg();
        assert!(a.push(&[1.0, -0.5, 0.0, 0.5], 1).is_none());
        assert_eq!(a.bins.len(), 1);
        let b = a.bins[0];
        assert!((b.min - (-0.5)).abs() < 1e-6);
        assert!((b.max - 1.0).abs() < 1e-6);
        assert!((b.rms - 0.612_372_4).abs() < 1e-5);
    }

    #[test]
    fn batch_produced_when_full() {
        let mut a = agg();
        // 2 个窗口：第一个 [1.0, -0.5, 0.0, 0.5]，第二个全 0
        assert!(a.push(&[1.0, -0.5, 0.0, 0.5], 1).is_none());
        let batch = a.push(&[0.0, 0.0, 0.0, 0.0], 1).expect("凑满一批");
        assert_eq!(batch.bins.len(), 2);
        assert_eq!(batch.device, "Test");
        assert_eq!(batch.sample_rate, 44100);
        assert_eq!(batch.window_ms, 10);
        assert_eq!(batch.duration_ms, 20);
        // 清空后继续累积下一批
        assert!(a.push(&[0.0, 0.0, 0.0, 0.0], 1).is_none());
    }

    #[test]
    fn multi_channel_mixed_to_mono() {
        let mut a = agg();
        // 2 声道、8 帧：前 4 帧 (1.0,-1.0) → mono 0.0，后 4 帧 (0,0) → mono 0.0
        // 凑满 2 个窗口 → 一批
        let batch = a
            .push(
                &[1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                2,
            )
            .expect("凑满一批");
        assert_eq!(batch.bins.len(), 2);
        for b in &batch.bins {
            assert!((b.min - 0.0).abs() < 1e-6);
            assert!((b.max - 0.0).abs() < 1e-6);
        }
    }

    #[test]
    fn window_spanning_across_pushes() {
        let mut a = agg();
        // 第一个窗口分两次 push：先 2 帧再 2 帧
        assert!(a.push(&[1.0, -0.5], 1).is_none());
        assert!(a.push(&[0.0, 0.5], 1).is_none());
        assert_eq!(a.bins.len(), 1);
        assert!(a.pending.is_empty(), "窗口完成后 pending 应清空");
    }

    #[test]
    fn partial_window_accumulates_pending() {
        let mut a = agg();
        // 5 个样本、1 声道：凑满第 1 个窗口（4 帧），剩 1 帧留到下一窗口
        a.push(&[0.0, 0.0, 0.0, 0.0, 1.0], 1);
        assert_eq!(a.bins.len(), 1);
        assert_eq!(a.pending.len(), 1);
        // 再补 3 帧完成第 2 个窗口 → 凑满一批；第 2 个 bin 含跨窗口的 1.0
        let batch = a.push(&[0.0, 0.0, 0.0], 1).expect("凑满一批");
        assert_eq!(batch.bins.len(), 2);
        assert_eq!(batch.bins[1].max, 1.0);
        assert_eq!(batch.bins[1].min, 0.0);
    }

    #[test]
    fn silence_has_zero_rms() {
        let mut a = agg();
        let batch = a.push(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 1).expect("凑满一批");
        for b in &batch.bins {
            assert_eq!(b.rms, 0.0);
            assert_eq!(b.min, 0.0);
            assert_eq!(b.max, 0.0);
        }
    }
}
