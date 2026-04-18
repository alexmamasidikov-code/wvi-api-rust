//! Detection engine — z-score + CUSUM + EWMA ensemble with Bayesian
//! change-point confidence for the critical metrics (wvi/hrv/stress).

use crate::sensitivity::{
    baseline,
    types::{Baseline, CusumState, DetectorState, Direction, EwmaState, Signal},
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub fn z_score_detect(value: f64, baseline: &Baseline) -> Option<(f64, Direction)> {
    if baseline.std < 1e-6 {
        return None;
    }
    let z = (value - baseline.mean) / baseline.std;
    if z.abs() < 2.0 {
        return None;
    }
    Some((z.abs(), if z > 0.0 { Direction::Up } else { Direction::Down }))
}

pub fn cusum_update(
    state: &mut CusumState,
    value: f64,
    baseline: &Baseline,
    k: f64,
    h: f64,
) -> Option<Direction> {
    if baseline.std < 1e-6 {
        return None;
    }
    let z = (value - baseline.mean) / baseline.std;
    state.s_plus = (state.s_plus + z - k).max(0.0);
    state.s_minus = (state.s_minus - z - k).max(0.0);
    if state.s_plus > h {
        state.s_plus = 0.0;
        return Some(Direction::Up);
    }
    if state.s_minus > h {
        state.s_minus = 0.0;
        return Some(Direction::Down);
    }
    None
}

pub fn ewma_update(
    state: &mut EwmaState,
    value: f64,
    baseline: &Baseline,
    lambda: f64,
    l_bound: f64,
) -> Option<Direction> {
    if baseline.std < 1e-6 {
        return None;
    }
    let z = (value - baseline.mean) / baseline.std;
    state.z = lambda * z + (1.0 - lambda) * state.z;
    let ewma_std = (lambda / (2.0 - lambda)).sqrt();
    if state.z.abs() > l_bound * ewma_std {
        return Some(if state.z > 0.0 { Direction::Up } else { Direction::Down });
    }
    None
}

pub fn bayesian_changepoint(history: &[f64], baseline: &Baseline) -> f64 {
    if history.len() < 5 {
        return 0.0;
    }
    let recent = &history[history.len().saturating_sub(5)..];
    let recent_mean = recent.iter().sum::<f64>() / recent.len() as f64;
    let diff = (recent_mean - baseline.mean).abs();
    let z = if baseline.std > 0.0 { diff / baseline.std } else { 0.0 };
    // Logistic-like mapping [0,1] — 0.5 at z=2, >0.9 at z=3.5.
    1.0 / (1.0 + (-1.5 * (z - 2.0)).exp())
}

pub fn ensemble_vote(
    z: Option<(f64, Direction)>,
    cusum: Option<Direction>,
    ewma: Option<Direction>,
) -> Option<(Direction, f64, Vec<&'static str>)> {
    let mut votes: Vec<(Direction, &'static str)> = vec![];
    let mut max_sigma: f64 = 0.0;
    if let Some((s, d)) = z {
        votes.push((d, "zscore"));
        max_sigma = max_sigma.max(s);
    }
    if let Some(d) = cusum {
        votes.push((d, "cusum"));
    }
    if let Some(d) = ewma {
        votes.push((d, "ewma"));
    }
    if votes.len() < 2 {
        return None;
    }
    let up = votes.iter().filter(|(d, _)| matches!(d, Direction::Up)).count();
    let down = votes.iter().filter(|(d, _)| matches!(d, Direction::Down)).count();
    let direction = if up >= down { Direction::Up } else { Direction::Down };
    let fired: Vec<&'static str> = votes
        .iter()
        .filter(|(d, _)| match (d, &direction) {
            (Direction::Up, Direction::Up) | (Direction::Down, Direction::Down) => true,
            _ => false,
        })
        .map(|(_, name)| *name)
        .collect();
    if fired.len() < 2 {
        return None;
    }
    Some((direction, max_sigma.max(2.0), fired))
}

pub fn classify_severity(sigma: f64, bayesian_conf: Option<f64>) -> &'static str {
    if bayesian_conf.unwrap_or(0.0) > 0.9 {
        return "high";
    }
    if sigma >= 4.0 {
        "high"
    } else if sigma >= 3.0 {
        "medium"
    } else {
        "low"
    }
}

pub async fn evaluate_bucket(
    pool: &PgPool,
    user_id: Uuid,
    metric: &str,
    ts: DateTime<Utc>,
    value: f64,
    activity_state: crate::sensitivity::types::ActivityState,
) -> sqlx::Result<Option<Signal>> {
    if !baseline::is_past_onboarding(pool, user_id).await? {
        return Ok(None);
    }

    let ctx = crate::sensitivity::types::ContextKey::from_ts(ts, activity_state);
    let Some(bl) = baseline::load(pool, user_id, metric, &ctx).await? else {
        return Ok(None);
    };
    if !bl.locked || bl.sample_count < 20 {
        return Ok(None);
    }

    let z = z_score_detect(value, &bl);
    let mut state: DetectorState = baseline::get_detector_state(pool, user_id, metric).await?;
    let cusum = cusum_update(&mut state.cusum, value, &bl, 0.5, 4.0);
    let ewma = ewma_update(&mut state.ewma, value, &bl, 0.2, 2.7);
    baseline::save_detector_state(pool, user_id, metric, &state).await?;

    let Some((direction, sigma, detectors)) = ensemble_vote(z, cusum, ewma) else {
        return Ok(None);
    };

    // Bayesian only for critical metrics — cheaper elsewhere.
    let bayesian = if matches!(metric, "wvi" | "hrv" | "stress") {
        let hist: Vec<(f64,)> = sqlx::query_as(
            "SELECT value FROM biometrics_1min
             WHERE user_id=$1 AND metric_type=$2
             ORDER BY ts DESC LIMIT 60",
        )
        .bind(user_id)
        .bind(metric)
        .fetch_all(pool)
        .await?;
        let hist_f: Vec<f64> = hist.into_iter().map(|(v,)| v).collect();
        Some(bayesian_changepoint(&hist_f, &bl))
    } else {
        None
    };

    let severity = classify_severity(sigma, bayesian);
    let direction_str = match direction {
        Direction::Up => "up",
        Direction::Down => "down",
    };

    // Simple placeholder rarity until real per-user signal-history index is built.
    let rarity = 0.5;

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO signals
            (id, user_id, ts, metric_type, context_key, deviation_sigma,
             direction, severity, detectors_fired, bayesian_confidence, rarity_percentile)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(user_id)
    .bind(ts)
    .bind(metric)
    .bind(ctx.as_str())
    .bind(sigma)
    .bind(direction_str)
    .bind(severity)
    .bind(serde_json::to_value(&detectors).unwrap())
    .bind(bayesian)
    .bind(rarity)
    .execute(pool)
    .await?;

    Ok(Some(Signal {
        id,
        user_id,
        ts,
        metric_type: metric.to_string(),
        context_key: ctx.as_str(),
        deviation_sigma: sigma,
        direction: direction_str.to_string(),
        severity: severity.to_string(),
        detectors_fired: serde_json::to_value(&detectors).unwrap(),
        bayesian_confidence: bayesian,
        rarity_percentile: Some(rarity),
        narrative: None,
        ack: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitivity::types::*;

    fn bl(mean: f64, std: f64) -> Baseline {
        Baseline {
            mean,
            std,
            p10: mean - std,
            p90: mean + std,
            sample_count: 100,
            locked: true,
        }
    }

    #[test]
    fn zscore_detects_outlier() {
        let b = bl(50.0, 5.0);
        let out = z_score_detect(70.0, &b);
        assert!(out.is_some());
        assert_eq!(out.unwrap().0, 4.0);
    }

    #[test]
    fn zscore_ignores_inrange() {
        let b = bl(50.0, 5.0);
        assert!(z_score_detect(52.0, &b).is_none());
    }

    #[test]
    fn cusum_detects_drift() {
        let b = bl(50.0, 5.0);
        let mut s = CusumState::default();
        // Feed a sustained 2σ shift — should trigger within a few iterations.
        let mut fired = None;
        for _ in 0..15 {
            if let Some(d) = cusum_update(&mut s, 60.0, &b, 0.5, 4.0) {
                fired = Some(d);
                break;
            }
        }
        assert!(fired.is_some());
    }

    #[test]
    fn ewma_detects_sustained_shift() {
        let b = bl(50.0, 5.0);
        let mut s = EwmaState::default();
        let mut fired = None;
        for _ in 0..20 {
            if let Some(d) = ewma_update(&mut s, 60.0, &b, 0.2, 2.7) {
                fired = Some(d);
                break;
            }
        }
        assert!(fired.is_some());
    }

    #[test]
    fn ensemble_two_of_three() {
        let z = Some((3.0, Direction::Up));
        let c = Some(Direction::Up);
        let e = None;
        assert!(ensemble_vote(z, c, e).is_some());
    }

    #[test]
    fn ensemble_only_one_fires() {
        let z = Some((3.0, Direction::Up));
        assert!(ensemble_vote(z, None, None).is_none());
    }

    #[test]
    fn bayesian_static_low() {
        let b = bl(50.0, 5.0);
        let conf = bayesian_changepoint(&[50.0, 50.0, 50.0, 50.0, 50.0], &b);
        assert!(conf < 0.3);
    }

    #[test]
    fn bayesian_shifted_high() {
        let b = bl(50.0, 5.0);
        let conf =
            bayesian_changepoint(&[50.0, 50.0, 50.0, 70.0, 72.0, 74.0, 73.0], &b);
        assert!(conf > 0.5);
    }

    #[test]
    fn severity_high_on_bayesian_spike() {
        assert_eq!(classify_severity(2.1, Some(0.95)), "high");
    }

    #[test]
    fn severity_by_sigma() {
        assert_eq!(classify_severity(4.5, None), "high");
        assert_eq!(classify_severity(3.2, None), "medium");
        assert_eq!(classify_severity(2.1, None), "low");
    }
}
