//! Emotion v2 — 5 aggregate metrics (agility, range, anchors, regulation,
//! diversity, contagion). `range` is the convex-hull area in (v,a) space
//! over the last 24h; `anchors` are the top-3 dwell emotions.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize)]
pub struct EmotionMetrics {
    pub agility: f64,
    pub range: f64,
    /// Top-3 emotion anchors as (label, dwell_ratio 0..1).
    pub anchors: Vec<(String, f64)>,
    pub regulation: f64,
    pub diversity: f64,
    pub contagion: f64,
}

pub async fn compute(pool: &PgPool, user_id: Uuid) -> anyhow::Result<EmotionMetrics> {
    let samples: Vec<(f64, f64, String)> = sqlx::query_as(
        "SELECT valence, arousal, primary_emotion FROM emotion_samples_1min
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '24 hours'
         ORDER BY ts ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let hull_area = convex_hull_area(&samples.iter().map(|(v, a, _)| (*v, *a)).collect::<Vec<_>>());

    // Dwell by primary label → top-3 anchors.
    use std::collections::HashMap;
    let mut dwell: HashMap<String, usize> = HashMap::new();
    for (_, _, p) in &samples {
        *dwell.entry(p.clone()).or_insert(0) += 1;
    }
    let total = samples.len().max(1) as f64;
    let mut anchors: Vec<(String, f64)> = dwell
        .into_iter()
        .map(|(k, v)| (k, v as f64 / total))
        .collect();
    anchors.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    anchors.truncate(3);

    // Diversity: Shannon entropy normalised by ln(18) (max possible).
    let entropy: f64 = anchors
        .iter()
        .map(|(_, p)| if *p > 0.0 { -p * p.ln() } else { 0.0 })
        .sum();
    let log_18 = (18.0_f64).ln();
    let diversity = (entropy / log_18 * 100.0).clamp(0.0, 100.0);

    // Agility / regulation / contagion — MVP constants; the upper-band spec
    // requires per-shift-recovery analysis + arousal decay fits + exogenous
    // correlation, deferred to a follow-up PR.
    let agility = 70.0;
    let regulation = 70.0;
    let contagion = 60.0;

    Ok(EmotionMetrics {
        agility,
        range: (hull_area / 4.0 * 100.0).clamp(0.0, 100.0),
        anchors,
        regulation,
        diversity,
        contagion,
    })
}

/// Andrew's monotone-chain convex-hull area. Used to score how much of the
/// (v,a) plane the user's emotions explored today. Clamped to 4.0 (the full
/// [-1,1]×[-1,1] square) so scaling stays bounded.
fn convex_hull_area(pts: &[(f64, f64)]) -> f64 {
    if pts.len() < 3 {
        return 0.0;
    }
    let mut pts = pts.to_vec();
    pts.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut hull: Vec<(f64, f64)> = vec![];
    for p in pts.iter() {
        while hull.len() >= 2
            && cross(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0
        {
            hull.pop();
        }
        hull.push(*p);
    }
    let lower_end = hull.len() + 1;
    for p in pts.iter().rev() {
        while hull.len() >= lower_end
            && cross(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0
        {
            hull.pop();
        }
        hull.push(*p);
    }
    hull.pop();
    let mut area = 0.0;
    for i in 0..hull.len() {
        let j = (i + 1) % hull.len();
        area += hull[i].0 * hull[j].1 - hull[j].0 * hull[i].1;
    }
    (area.abs() / 2.0).min(4.0)
}

fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_zero_when_lt_3_points() {
        assert_eq!(convex_hull_area(&[]), 0.0);
        assert_eq!(convex_hull_area(&[(0.0, 0.0)]), 0.0);
        assert_eq!(convex_hull_area(&[(0.0, 0.0), (1.0, 0.0)]), 0.0);
    }

    #[test]
    fn hull_of_unit_square_is_1() {
        let pts = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let area = convex_hull_area(&pts);
        assert!((area - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hull_clamps_at_4() {
        let pts = vec![(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        let area = convex_hull_area(&pts);
        assert!((area - 4.0).abs() < 1e-6);
    }
}
