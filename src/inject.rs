//! 上报 lhm `/inject` 端点：注入字段 + `__fields` 字段定义，失败重试。
//!
//! 契约（与 lhm 服务端 `Program.cs` 对齐）：
//! - POST body 为 `{ 字段名: 值, ..., "__fields": { 字段名: 定义 } }`；
//! - 字段定义要求 `label`（非空字符串）与 `unit` 必填，`min`/`max` 可选数值；
//! - 字段 5 秒 TTL 过期，因此每次上报都要携带，且上报间隔必须小于 5 秒。

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;
use ureq::Agent;

use crate::waveform::WaveformBatch;

/// 波形字段名（合并进 lhm Standard 的动态字段）。
pub const FIELD_WAVEFORM: &str = "AudioWaveform";
/// 当前响度字段名（整批 RMS 均值）。
pub const FIELD_LEVEL: &str = "AudioLevel";

const FIELDS_DEF: &str = r#"{
  "AudioWaveform": { "label": "音频波形", "unit": "振幅", "min": -1, "max": 1, "group": "音频" },
  "AudioLevel": { "label": "音频响度", "unit": "RMS", "min": 0, "max": 1, "group": "音频" }
}"#;

pub fn build_agent() -> Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .build();
    Agent::new_with_config(config)
}

/// 序列化一批波形为 lhm `/inject` 请求体（值 + `__fields` 定义，每次携带以刷新 TTL）。
pub fn build_payload(batch: &WaveformBatch) -> serde_json::Value {
    let level = batch
        .bins
        .iter()
        .map(|b| f64::from(b.rms))
        .sum::<f64>()
        / batch.bins.len().max(1) as f64;
    json!({
        FIELD_WAVEFORM: batch
            .bins
            .iter()
            .map(|b| json!({ "min": b.min, "max": b.max, "rms": b.rms }))
            .collect::<Vec<_>>(),
        FIELD_LEVEL: level,
        "__fields": serde_json::from_str::<serde_json::Value>(FIELDS_DEF)
            .expect("静态字段定义必须合法"),
    })
}

/// POST 一批波形到 lhm `/inject`，最多重试 3 次（指数退避），仍失败则返回错误。
pub fn post_batch(agent: &Agent, url: &str, batch: &WaveformBatch) -> Result<()> {
    let payload = build_payload(batch);
    let mut delay = Duration::from_millis(400);
    let mut last = None;
    for _ in 1..=3 {
        match agent
            .post(url)
            .header("Content-Type", "application/json")
            .send_json(&payload)
        {
            Ok(resp) if resp.status() == 200 => return Ok(()),
            Ok(resp) => last = Some(anyhow::anyhow!("lhm 返回状态 {}", resp.status())),
            Err(e) => last = Some(anyhow::anyhow!("请求失败: {e}")),
        }
        std::thread::sleep(delay);
        delay *= 2;
    }
    Err(last.expect("至少尝试过一次")).with_context(|| format!("POST {url} 重试 3 次后仍失败"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::{Bin, WaveformBatch};

    fn batch() -> WaveformBatch {
        WaveformBatch {
            device: "Test".into(),
            sample_rate: 44100,
            window_ms: 20,
            duration_ms: 40,
            bins: vec![
                Bin { min: -0.8, max: 0.9, rms: 0.5 },
                Bin { min: -0.2, max: 0.1, rms: 0.1 },
            ],
        }
    }

    /// 与 lhm `Program.cs` 的 `ValidateFieldDef` 对齐的校验。
    fn validate_field_def(def: &serde_json::Value) -> bool {
        let obj = match def.as_object() {
            Some(o) => o,
            None => return false,
        };
        let label = obj
            .get("label")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("");
        if label.is_empty() {
            return false;
        }
        if obj.get("unit").is_none() {
            return false;
        }
        for key in ["min", "max"] {
            if let Some(v) = obj.get(key) {
                // lhm 用 TryGetValue<double>，整数数字同样能转 double。
                if !v.is_number() {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn payload_matches_lhm_inject_contract() {
        let payload = build_payload(&batch());
        let obj = payload.as_object().expect("body 必须是对象");

        let waveform = obj.get(FIELD_WAVEFORM).and_then(|v| v.as_array());
        assert_eq!(waveform.map(|a| a.len()), Some(2));
        for bin in waveform.unwrap() {
            assert!(bin.get("min").is_some());
            assert!(bin.get("max").is_some());
            assert!(bin.get("rms").is_some());
        }

        // AudioLevel = 整批 RMS 均值 (0.5+0.1)/2 = 0.3
        assert!((obj.get(FIELD_LEVEL).unwrap().as_f64().unwrap() - 0.3).abs() < 1e-9);

        let fields = obj.get("__fields").and_then(|v| v.as_object());
        assert!(fields.is_some());
        for (name, def) in fields.unwrap() {
            assert!(
                validate_field_def(def),
                "字段 {name} 定义不满足 lhm ValidateFieldDef: {def}"
            );
        }
    }

    #[test]
    fn empty_bins_level_is_zero() {
        let mut b = batch();
        b.bins.clear();
        let payload = build_payload(&b);
        assert_eq!(payload.get(FIELD_LEVEL).unwrap().as_f64(), Some(0.0));
        assert!(payload.get(FIELD_WAVEFORM).unwrap().as_array().unwrap().is_empty());
    }
}
