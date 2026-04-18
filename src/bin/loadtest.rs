// Load test binary to verify 1M-scale fixes.
// Spawns N concurrent tokio tasks, each POSTing /biometrics/sync batches.
// Tracks total requests, errors, p50/p95/p99 latency.

use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::sleep;

struct Args {
    users: u64,
    duration: u64,
    target: String,
    token: String,
}

fn parse_args() -> Args {
    let mut users = 1000u64;
    let mut duration = 60u64;
    let mut target = "http://localhost:3000".to_string();
    let mut token = env::var("LOADTEST_TOKEN").unwrap_or_default();

    let argv: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--users" => { users = argv[i + 1].parse().unwrap_or(1000); i += 2; }
            "--duration" => { duration = argv[i + 1].parse().unwrap_or(60); i += 2; }
            "--target" => { target = argv[i + 1].clone(); i += 2; }
            "--token" => { token = argv[i + 1].clone(); i += 2; }
            _ => { i += 1; }
        }
    }
    Args { users, duration, target, token }
}

fn build_batch() -> serde_json::Value {
    let now = chrono::Utc::now();
    let mut records = Vec::with_capacity(10);
    for i in 0..10 {
        let ts = now - chrono::Duration::seconds(i);
        let kind = match i % 3 {
            0 => ("heart_rate", json!({ "bpm": 70 + i })),
            1 => ("hrv", json!({ "rmssd": 40 + i })),
            _ => ("spo2", json!({ "percentage": 96 + (i % 4) })),
        };
        records.push(json!({
            "type": kind.0,
            "timestamp": ts.to_rfc3339(),
            "data": kind.1,
        }));
    }
    json!({ "deviceId": "loadtest-device", "records": records })
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args = parse_args();
    let client = Client::builder()
        .pool_max_idle_per_host(256)
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");

    let url = format!("{}/api/v1/biometrics/sync", args.target.trim_end_matches('/'));
    let total = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let latencies: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::with_capacity(1_000_000)));
    let stop_at = Instant::now() + Duration::from_secs(args.duration);

    println!(
        "Load test: {} users × {}s, target={}",
        args.users, args.duration, args.target
    );

    let mut handles = Vec::with_capacity(args.users as usize);
    for uid in 0..args.users {
        let client = client.clone();
        let url = url.clone();
        let token = args.token.clone();
        let total = total.clone();
        let errors = errors.clone();
        let latencies = latencies.clone();
        handles.push(tokio::spawn(async move {
            while Instant::now() < stop_at {
                let body = build_batch();
                let t0 = Instant::now();
                let mut req = client.post(&url).json(&body);
                if !token.is_empty() {
                    req = req.bearer_auth(&token);
                }
                req = req.header("X-User-Hint", uid.to_string());
                let res = req.send().await;
                let elapsed = t0.elapsed().as_millis() as u64;
                total.fetch_add(1, Ordering::Relaxed);
                match res {
                    Ok(r) if r.status().is_success() => {
                        let mut l = latencies.lock().await;
                        if l.len() < 1_000_000 { l.push(elapsed); }
                    }
                    _ => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
                sleep(Duration::from_secs(2)).await;
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let total_n = total.load(Ordering::Relaxed);
    let err_n = errors.load(Ordering::Relaxed);
    let ok_n = total_n.saturating_sub(err_n);
    let mut lats = latencies.lock().await.clone();
    lats.sort_unstable();
    let pct = |p: f64| -> u64 {
        if lats.is_empty() { return 0; }
        let idx = ((lats.len() as f64 - 1.0) * p).round() as usize;
        lats[idx.min(lats.len() - 1)]
    };
    let ok_pct = if total_n > 0 { (ok_n as f64 / total_n as f64) * 100.0 } else { 0.0 };
    let err_pct = if total_n > 0 { (err_n as f64 / total_n as f64) * 100.0 } else { 0.0 };

    println!(
        "Load test: {} users × {}s = {} requests",
        args.users, args.duration, total_n
    );
    println!("Success: {} ({:.2}%)", ok_n, ok_pct);
    println!("Errors:  {} ({:.2}%)", err_n, err_pct);
    println!(
        "Latency: p50={}ms p95={}ms p99={}ms",
        pct(0.50),
        pct(0.95),
        pct(0.99)
    );
}
