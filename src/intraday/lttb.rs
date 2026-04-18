use crate::intraday::types::ChartPoint;
use chrono::{DateTime, Utc};

pub fn downsample(points: &[ChartPoint], target: usize) -> Vec<ChartPoint> {
    if points.len() <= target || target < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(target);
    let bucket_size = (points.len() - 2) as f64 / (target - 2) as f64;

    out.push(points[0].clone());
    let mut a_idx: usize = 0;

    for i in 0..(target - 2) {
        let bucket_start = ((i as f64 + 1.0) * bucket_size + 1.0) as usize;
        let bucket_end = ((i as f64 + 2.0) * bucket_size + 1.0) as usize;
        let bucket_end = bucket_end.min(points.len());

        // average point in next bucket
        let next_start = bucket_end;
        let next_end = ((i as f64 + 3.0) * bucket_size + 1.0) as usize;
        let next_end = next_end.min(points.len());
        let (avg_t, avg_v) = average(&points[next_start..next_end]);

        // find point in current bucket with max triangle area
        let a = &points[a_idx];
        let mut max_area = -1.0;
        let mut max_idx = bucket_start;
        for j in bucket_start..bucket_end {
            let area = triangle_area(a.ts, a.value, points[j].ts, points[j].value, avg_t, avg_v);
            if area > max_area {
                max_area = area;
                max_idx = j;
            }
        }
        out.push(points[max_idx].clone());
        a_idx = max_idx;
    }
    out.push(points[points.len() - 1].clone());
    out
}

fn average(pts: &[ChartPoint]) -> (DateTime<Utc>, f64) {
    if pts.is_empty() {
        return (Utc::now(), 0.0);
    }
    let t_ms: i64 = pts.iter().map(|p| p.ts.timestamp_millis()).sum::<i64>() / pts.len() as i64;
    let v: f64 = pts.iter().map(|p| p.value).sum::<f64>() / pts.len() as f64;
    (DateTime::from_timestamp_millis(t_ms).unwrap_or_else(Utc::now), v)
}

fn triangle_area(
    t1: DateTime<Utc>,
    v1: f64,
    t2: DateTime<Utc>,
    v2: f64,
    t3: DateTime<Utc>,
    v3: f64,
) -> f64 {
    let x1 = t1.timestamp_millis() as f64 / 1000.0;
    let x2 = t2.timestamp_millis() as f64 / 1000.0;
    let x3 = t3.timestamp_millis() as f64 / 1000.0;
    0.5 * ((x1 - x3) * (v2 - v1) - (x1 - x2) * (v3 - v1)).abs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn mkpt(i: i64, v: f64) -> ChartPoint {
        ChartPoint {
            ts: Utc::now() + Duration::minutes(i),
            value: v,
            min: None,
            max: None,
        }
    }

    #[test]
    fn preserves_when_under_target() {
        let pts = vec![mkpt(0, 1.0), mkpt(1, 2.0)];
        let out = downsample(&pts, 10);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn downsamples_to_target() {
        let pts: Vec<ChartPoint> = (0..1000).map(|i| mkpt(i as i64, (i as f64).sin())).collect();
        let out = downsample(&pts, 100);
        assert_eq!(out.len(), 100);
    }

    #[test]
    fn preserves_endpoints() {
        let pts: Vec<ChartPoint> = (0..100).map(|i| mkpt(i as i64, i as f64)).collect();
        let out = downsample(&pts, 10);
        assert_eq!(out.first().unwrap().value, 0.0);
        assert_eq!(out.last().unwrap().value, 99.0);
    }

    #[test]
    fn preserves_peak() {
        let mut pts: Vec<ChartPoint> = (0..100).map(|i| mkpt(i as i64, 1.0)).collect();
        pts[50].value = 100.0;
        let out = downsample(&pts, 20);
        let has_peak = out.iter().any(|p| p.value > 90.0);
        assert!(has_peak, "LTTB should preserve sharp peak");
    }
}
