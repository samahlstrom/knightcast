use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub asset_root: String,
    pub port: u16,
    pub smoothing_ms: f32,
    pub mouth_gain: f32,
    pub mouth_baseline: f32,
    pub grace_ms: f32,
    pub return_ms: f32,
    pub model_scale: f32,
    pub body_scale: f32,
    pub offset: [f32; 2],
    pub base_degrees: [f32; 3],
    pub pivot: [f32; 3],
    pub zoom: f32,
    pub background: [f32; 4],
    pub neutral: [f32; 4],
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            asset_root: String::new(),
            port: 39539,
            smoothing_ms: 25.,
            mouth_gain: 3.8,
            mouth_baseline: 0.,
            grace_ms: 250.,
            return_ms: 400.,
            model_scale: 1.,
            body_scale: 1.,
            offset: [0., 0.],
            base_degrees: [0.; 3],
            pivot: [0., 2.125, 0.],
            zoom: 1.,
            background: [0.055, 0.065, 0.09, 1.],
            neutral: [0., 0., 0., 1.],
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if !self.mouth_gain.is_finite()
            || !(0.1..=10.).contains(&self.mouth_gain)
            || !self.mouth_baseline.is_finite()
            || !(0. ..=1.).contains(&self.mouth_baseline)
        {
            bail!("invalid mouth sensitivity/baseline")
        };
        if self.version != 1 {
            bail!("unsupported config version")
        };
        if self.port == 0 {
            bail!("port must be nonzero")
        };
        for x in [self.smoothing_ms, self.grace_ms, self.return_ms] {
            if !x.is_finite() || !(0. ..=60000.).contains(&x) {
                bail!("timing must be finite and between 0 and 60000 ms")
            }
        }
        for x in [self.model_scale, self.body_scale, self.zoom] {
            if !x.is_finite() || !(0.1..=10.).contains(&x) {
                bail!("scales must be between 0.1 and 10")
            }
        }
        for x in self
            .offset
            .iter()
            .chain(&self.base_degrees)
            .chain(&self.pivot)
        {
            if !x.is_finite() || x.abs() > 10000. {
                bail!("invalid transform")
            }
        }
        for x in self.background {
            if !x.is_finite() || !(0. ..=1.).contains(&x) {
                bail!("invalid background")
            }
        }
        let q = glam::Quat::from_array(self.neutral);
        if !q.is_finite() || !q.length().is_finite() || q.length() < 0.000001 {
            bail!("invalid calibration")
        };
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let c: Self = serde_json::from_slice(&std::fs::read(path).context("read configuration")?)
            .context("parse configuration")?;
        c.validate()?;
        Ok(c)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension(format!("tmp-{}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    }
}
pub fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("knightcast/config.json")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_invalid_preserves_file() {
        let p = std::env::temp_dir().join(format!("knightcast-test-{}.json", std::process::id()));
        let c = Config::default();
        c.save(&p).unwrap();
        assert_eq!(c, Config::load(&p).unwrap());
        let mut bad = c.clone();
        bad.zoom = 0.;
        assert!(bad.save(&p).is_err());
        assert_eq!(c, Config::load(&p).unwrap());
        std::fs::remove_file(p).unwrap();
    }
}
