use axum::Json;
use serde_json::json;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

/// Deterministic pseudo-random WVI score in [65, 90] based on DID + member index.
fn seeded_wvi(did: &str, idx: u8) -> u8 {
    let mut h: u32 = 2166136261;
    for b in did.as_bytes() { h = h.wrapping_mul(16777619) ^ (*b as u32); }
    h = h.wrapping_mul(16777619) ^ (idx as u32);
    65 + (h % 26) as u8
}

fn seeded_range(did: &str, idx: u8, salt: u8, lo: u32, hi: u32) -> u32 {
    let mut h: u32 = 2166136261;
    for b in did.as_bytes() { h = h.wrapping_mul(16777619) ^ (*b as u32); }
    h = h.wrapping_mul(16777619) ^ (idx as u32);
    h = h.wrapping_mul(16777619) ^ (salt as u32);
    lo + (h % (hi - lo + 1))
}

fn emotion_for(wvi: u8) -> &'static str {
    if wvi >= 85 { "Joy" }
    else if wvi >= 78 { "Calm" }
    else if wvi >= 72 { "Focus" }
    else if wvi >= 68 { "Tired" }
    else { "Stress" }
}

fn members_for(did: &str) -> Vec<serde_json::Value> {
    let roster: [(&str, &str); 4] = [
        ("Alex",   "You"),
        ("Mom",    "Mother"),
        ("Dad",    "Father"),
        ("Sister", "Sister"),
    ];
    roster.iter().enumerate().map(|(i, (name, rel))| {
        let idx = i as u8;
        let wvi = seeded_wvi(did, idx);
        let online = seeded_range(did, idx, 1, 0, 3) != 0; // ~75% online
        let last_seen = if online { 0 } else { seeded_range(did, idx, 2, 5, 240) };
        json!({
            "id": format!("fm_{}", idx + 1),
            "name": name,
            "relation": rel,
            "wvi": wvi,
            "emotion": emotion_for(wvi),
            "online": online,
            "last_seen_minutes": last_seen,
        })
    }).collect()
}

/// GET /api/v1/family/members
pub async fn members(user: AuthUser) -> AppResult<Json<serde_json::Value>> {
    let list = members_for(&user.privy_did);
    Ok(Json(json!({ "success": true, "data": { "members": list } })))
}

/// GET /api/v1/family/average
pub async fn average(user: AuthUser) -> AppResult<Json<serde_json::Value>> {
    let list = members_for(&user.privy_did);
    let count = list.len() as u32;
    let sum: u32 = list.iter().map(|m| m["wvi"].as_u64().unwrap_or(0) as u32).sum();
    let avg = if count == 0 { 0.0 } else { sum as f32 / count as f32 };
    Ok(Json(json!({
        "success": true,
        "data": { "average_wvi": (avg * 10.0).round() / 10.0, "member_count": count }
    })))
}

/// GET /api/v1/family/alerts
pub async fn alerts(user: AuthUser) -> AppResult<Json<serde_json::Value>> {
    let list = members_for(&user.privy_did);
    let mut out: Vec<serde_json::Value> = Vec::new();
    for m in list.iter().skip(1) { // skip "You"
        let wvi = m["wvi"].as_u64().unwrap_or(100) as u8;
        let name = m["name"].as_str().unwrap_or("Family");
        if wvi < 72 {
            out.push(json!({
                "member": name,
                "type": "stress",
                "message": format!("{}'s stress is elevated today", name),
                "severity": if wvi < 68 { "high" } else { "medium" }
            }));
        }
        if out.len() >= 2 { break; }
    }
    if out.is_empty() {
        // ensure at least one item so UI has something to render
        if let Some(m) = list.get(1) {
            let name = m["name"].as_str().unwrap_or("Mom");
            out.push(json!({
                "member": name,
                "type": "info",
                "message": format!("{}'s WVI is stable today", name),
                "severity": "low"
            }));
        }
    }
    Ok(Json(json!({ "success": true, "data": { "alerts": out } })))
}
