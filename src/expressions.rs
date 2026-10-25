//! Port of the user's stream-kit surprise trigger. Lip opening is unamplified;
//! ordinary talking, eye widening alone, stale samples and winks cannot fire it.
pub struct SurpriseMotion {
    pub level: f32,
    armed: bool,
    qualifying_since: Option<f64>,
    relaxed_since: Option<f64>,
    until: f64,
    last_time: f64,
}
impl Default for SurpriseMotion {
    fn default() -> Self {
        Self {
            level: 0.,
            armed: true,
            qualifying_since: None,
            relaxed_since: None,
            until: f64::NEG_INFINITY,
            last_time: f64::NEG_INFINITY,
        }
    }
}
impl SurpriseMotion {
    pub fn step(
        &mut self,
        dt: f32,
        now: f64,
        fresh: bool,
        mouth: f32,
        wide: [f32; 2],
        blink: [f32; 2],
    ) {
        if now < self.last_time {
            *self = Self::default();
        }
        self.last_time = now;
        let qualifying = fresh
            && mouth >= 0.22
            && wide.iter().all(|v| *v >= 0.28)
            && blink.iter().all(|v| *v < 0.3);
        if qualifying {
            self.relaxed_since = None;
            let since = self.qualifying_since.get_or_insert(now);
            if self.armed && now - *since >= 0.12 {
                self.until = now + 0.65;
                self.armed = false;
            } else if !self.armed && now < self.until {
                self.until = self.until.max(now + 0.18);
            }
        } else {
            self.qualifying_since = None;
            if !fresh || mouth < 0.12 || wide[0].min(wide[1]) < 0.15 {
                let since = self.relaxed_since.get_or_insert(now);
                if now - *since > 0.25 && now >= self.until {
                    self.armed = true;
                }
            } else {
                self.relaxed_since = None;
            }
        }
        if !fresh {
            self.until = self.until.min(now);
        }
        let active = now < self.until;
        let target = if active { 1. } else { 0. };
        self.level +=
            (target - self.level) * (1. - (-dt.max(0.) / if active { 0.065 } else { 0.14 }).exp());
        if self.level < 0.001 {
            self.level = 0.;
        }
    }
}

pub fn wide_response(raw: f32, baseline: f32) -> f32 {
    ((raw - baseline) / (1. - baseline).max(0.2)).clamp(0., 1.)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_mouth_both_wide_eyes_and_sustained_fresh_data() {
        for (mouth, wide, blink, fresh) in [
            (0.6, [0.; 2], [0.; 2], true),
            (0.0, [0.8; 2], [0.; 2], true),
            (0.1, [0.8; 2], [0.; 2], true),
            (0.6, [0.8, 0.1], [0.; 2], true),
            (0.6, [0.8; 2], [0.5, 0.], true),
            (0.6, [0.8; 2], [0.; 2], false),
        ] {
            let mut motion = SurpriseMotion::default();
            for i in 0..120 {
                motion.step(1. / 60., i as f64 / 60., fresh, mouth, wide, blink);
            }
            assert_eq!(motion.level, 0.);
        }
        let mut motion = SurpriseMotion::default();
        for i in 0..6 {
            motion.step(1. / 60., i as f64 / 60., true, 0.6, [0.8; 2], [0.; 2]);
        }
        assert_eq!(motion.level, 0., "A brief spike must not fire");
        for i in 6..60 {
            motion.step(1. / 60., i as f64 / 60., true, 0.6, [0.8; 2], [0.; 2]);
        }
        assert!(motion.level > 0.99);
        for i in 60..150 {
            motion.step(1. / 60., i as f64 / 60., false, 0.6, [0.8; 2], [0.; 2]);
        }
        assert_eq!(motion.level, 0., "Camera loss must clear the effect");
        for i in 150..210 {
            motion.step(1. / 60., i as f64 / 60., true, 0.6, [0.8; 2], [0.; 2]);
        }
        assert!(motion.level > 0.99, "Relaxation must rearm");
    }
    #[test]
    fn eye_widening_is_relative_to_resting_calibration() {
        assert_eq!(wide_response(0.38, 0.38), 0.);
        assert_eq!(wide_response(0.20, 0.38), 0.);
        assert!((wide_response(0.8, 0.38) - 0.677419).abs() < 0.00001);
    }
}
