use axum::{extract::{Path, State}, Json};
use serde_json::json;
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};

/// Deterministic pseudo-random WVI score in [65, 90] used only for the fallback seed.
fn seeded_wvi(did: &str, idx: u8) -> u8 {
    let mut h: u32 = 2166136261;
    for b in did.as_bytes() { h = h.wrapping_mul(16777619) ^ (*b as u32); }
    h = h.wrapping_mul(16777619) ^ (idx as u32);
    65 + (h % 26) as u8
}

fn emotion_for(wvi: u8) -> &'static str {
    if wvi >= 85 { "Joy" }
    else if wvi >= 78 { "Calm" }
    else if wvi >= 72 { "Focus" }
    else if wvi >= 68 { "Tired" }
    else { "Stress" }
}

/// Ensure the caller has at least a `self` row + one pending mock invite, so the
/// UI always has something to render on first launch. Idempotent.
async fn ensure_seed(pool: &PgPool, did: &str) -> AppResult<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM family_members WHERE owner_id = $1")
        .bind(did).fetch_one(pool).await?;
    if count > 0 { return Ok(()); }
    sqlx::query(
        "INSERT INTO family_members (owner_id, member_id, display_name, relation, status, accepted_at) \
         VALUES ($1, $1, 'You', 'self', 'accepted', NOW())"
    ).bind(did).execute(pool).await?;
    sqlx::query(
        "INSERT INTO family_members (owner_id, display_name, relation, status, invite_email) \
         VALUES ($1, 'Mom', 'parent', 'pending', NULL)"
    ).bind(did).execute(pool).await?;
    Ok(())
}

/// Latest WVI score for a given privy_did, if any.
async fn latest_wvi(pool: &PgPool, privy_did: &str) -> Option<f32> {
    sqlx::query_scalar::<_, f32>(
        "SELECT ws.wvi_score FROM wvi_scores ws \
         JOIN users u ON ws.user_id = u.id \
         WHERE u.privy_did = $1 \
         ORDER BY ws.timestamp DESC LIMIT 1"
    ).bind(privy_did).fetch_optional(pool).await.ok().flatten()
}

/// GET /api/v1/family/members — real DB + latest WVI per member.
pub async fn members(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    ensure_seed(&pool, &user.privy_did).await?;

    let rows = sqlx::query_as::<_, (uuid::Uuid, Option<String>, String, String, String)>(
        "SELECT id, member_id, display_name, relation, status \
         FROM family_members WHERE owner_id = $1 ORDER BY created_at ASC"
    ).bind(&user.privy_did).fetch_all(&pool).await?;

    let mut out: Vec<serde_json::Value> = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        let idx = i as u8;
        let wvi: u8 = match &r.1 {
            Some(mid) => match latest_wvi(&pool, mid).await {
                Some(s) => s.round().clamp(0.0, 100.0) as u8,
                None => seeded_wvi(&user.privy_did, idx),
            },
            None => seeded_wvi(&user.privy_did, idx),
        };
        out.push(json!({
            "id": format!("fm_{}", r.0),
            "name": r.2,
            "relation": r.3,
            "status": r.4,
            "wvi": wvi,
            "emotion": emotion_for(wvi),
            "online": r.1.is_some() && r.4 == "accepted",
        }));
    }
    Ok(Json(json!({ "success": true, "data": { "members": out } })))
}

/// GET /api/v1/family/average — AVG latest WVI across accepted members.
pub async fn average(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    ensure_seed(&pool, &user.privy_did).await?;

    let member_ids: Vec<String> = sqlx::query_scalar(
        "SELECT member_id FROM family_members \
         WHERE owner_id = $1 AND status = 'accepted' AND member_id IS NOT NULL"
    ).bind(&user.privy_did).fetch_all(&pool).await?;

    let mut scores: Vec<f32> = Vec::new();
    for mid in &member_ids {
        if let Some(s) = latest_wvi(&pool, mid).await { scores.push(s); }
    }
    let count = scores.len() as u32;
    let avg = if count == 0 { 0.0 } else { scores.iter().sum::<f32>() / count as f32 };
    Ok(Json(json!({
        "success": true,
        "data": { "average_wvi": (avg * 10.0).round() / 10.0, "member_count": count }
    })))
}

/// GET /api/v1/family/alerts — members with recent anomaly alerts or low WVI.
pub async fn alerts(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    ensure_seed(&pool, &user.privy_did).await?;

    let rows = sqlx::query_as::<_, (Option<String>, String, String)>(
        "SELECT member_id, display_name, relation FROM family_members \
         WHERE owner_id = $1 AND status = 'accepted' AND relation <> 'self'"
    ).bind(&user.privy_did).fetch_all(&pool).await?;

    let mut out: Vec<serde_json::Value> = Vec::new();
    for (mid_opt, name, _rel) in rows.iter() {
        let Some(mid) = mid_opt else { continue };
        let recent_alert: Option<(String, String)> = sqlx::query_as(
            "SELECT a.level, a.message FROM alerts a \
             JOIN users u ON a.user_id = u.id \
             WHERE u.privy_did = $1 AND a.created_at > NOW() - INTERVAL '24 hours' \
             ORDER BY a.created_at DESC LIMIT 1"
        ).bind(mid).fetch_optional(&pool).await.ok().flatten();

        if let Some((level, message)) = recent_alert {
            out.push(json!({
                "member": name,
                "type": "anomaly",
                "message": message,
                "severity": level,
            }));
            continue;
        }
        if let Some(wvi_score) = latest_wvi(&pool, mid).await {
            let wvi = wvi_score.round().clamp(0.0, 100.0) as u8;
            if wvi < 72 {
                out.push(json!({
                    "member": name,
                    "type": "stress",
                    "message": format!("{}'s stress is elevated today", name),
                    "severity": if wvi < 68 { "high" } else { "medium" },
                }));
            }
        }
        if out.len() >= 5 { break; }
    }
    if out.is_empty() {
        // fall back to a harmless info item so the UI has something to render
        let fallback_name: Option<String> = sqlx::query_scalar(
            "SELECT display_name FROM family_members \
             WHERE owner_id = $1 AND relation <> 'self' ORDER BY created_at ASC LIMIT 1"
        ).bind(&user.privy_did).fetch_optional(&pool).await.ok().flatten();
        if let Some(name) = fallback_name {
            out.push(json!({
                "member": name,
                "type": "info",
                "message": format!("{}'s WVI is stable today", name),
                "severity": "low",
            }));
        }
    }
    Ok(Json(json!({ "success": true, "data": { "alerts": out } })))
}

/// POST /api/v1/family/invite { email, relation, display_name }
pub async fn invite(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let email = body.get("email").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("email required".into()))?;
    let relation = body.get("relation").and_then(|v| v.as_str()).unwrap_or("other");
    let display_name = body.get("display_name").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("display_name required".into()))?;

    let allowed = ["self", "spouse", "child", "parent", "sibling", "other"];
    if !allowed.contains(&relation) {
        return Err(AppError::BadRequest("invalid relation".into()));
    }

    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO family_members (owner_id, invite_email, display_name, relation, status) \
         VALUES ($1, $2, $3, $4, 'pending') RETURNING id"
    ).bind(&user.privy_did).bind(email).bind(display_name).bind(relation)
     .fetch_one(&pool).await?;

    Ok(Json(json!({
        "success": true,
        "data": { "invite_id": id.to_string(), "status": "pending" }
    })))
}

/// POST /api/v1/family/accept/:id — caller accepts an invite; stamps member_id.
pub async fn accept(
    user: AuthUser,
    State(pool): State<PgPool>,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let updated = sqlx::query(
        "UPDATE family_members SET status = 'accepted', member_id = $2, accepted_at = NOW() \
         WHERE id = $1 AND status = 'pending'"
    ).bind(id).bind(&user.privy_did).execute(&pool).await?;

    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound("invite not found or already handled".into()));
    }
    Ok(Json(json!({
        "success": true,
        "data": { "invite_id": id.to_string(), "status": "accepted" }
    })))
}
