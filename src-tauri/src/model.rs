use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionStatus {
    Starting,
    NeedsCodex,
    NeedsAuth,
    Ready,
    Reconnecting,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateDto {
    pub status: ConnectionStatus,
    pub snapshot: Option<UsageSnapshot>,
    pub message: Option<String>,
    pub codex_version: Option<String>,
    pub codex_path: Option<String>,
    pub autostart_enabled: bool,
    pub expanded: bool,
    pub updating: bool,
}

impl Default for AppStateDto {
    fn default() -> Self {
        Self {
            status: ConnectionStatus::Starting,
            snapshot: None,
            message: None,
            codex_version: None,
            codex_path: None,
            autostart_enabled: true,
            expanded: false,
            updating: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub quota_groups: Vec<QuotaGroup>,
    pub token_activity: TokenActivity,
    pub credits: Option<CreditState>,
    pub updated_at: i64,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaGroup {
    pub id: String,
    pub name: String,
    pub primary: bool,
    pub plan_type: Option<String>,
    pub windows: Vec<QuotaWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub key: String,
    pub label: String,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub window_duration_mins: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenActivity {
    /// Decimal strings preserve the app-server's bigint values across Tauri IPC.
    pub today_tokens: Option<String>,
    pub lifetime_tokens: Option<String>,
    pub peak_daily_tokens: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditState {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
    pub spend_control_reached: bool,
    pub individual_limit: Option<SpendControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendControl {
    pub limit: String,
    pub used: String,
    pub remaining_percent: f64,
    pub resets_at: i64,
}

pub fn normalize_snapshot(
    rate_limit_result: &Value,
    usage_result: Option<&Value>,
    today: &str,
    updated_at: i64,
) -> Result<UsageSnapshot, String> {
    let legacy = rate_limit_result
        .get("rateLimits")
        .filter(|value| value.is_object());
    let buckets = rate_limit_result
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object);

    let mut groups = Vec::new();
    let mut seen = HashSet::new();
    let main = buckets
        .and_then(|map| map.get("codex"))
        .filter(|value| value.is_object())
        .or(legacy)
        .ok_or_else(|| "Codex returned no rate-limit snapshot".to_string())?;

    let main_id = main
        .get("limitId")
        .and_then(Value::as_str)
        .unwrap_or("codex")
        .to_string();
    groups.push(normalize_group(&main_id, main, true));
    seen.insert(main_id.clone());
    seen.insert("codex".to_string());

    if let Some(buckets) = buckets {
        for (bucket_id, bucket) in buckets {
            if seen.contains(bucket_id) || !bucket.is_object() {
                continue;
            }
            let normalized_id = bucket
                .get("limitId")
                .and_then(Value::as_str)
                .unwrap_or(bucket_id);
            if seen.insert(normalized_id.to_string()) {
                groups.push(normalize_group(normalized_id, bucket, false));
            }
        }
    }

    groups.retain(|group| !group.windows.is_empty());
    let token_activity = usage_result
        .filter(|value| value.is_object())
        .map(|value| normalize_token_activity(value, today))
        .unwrap_or_default();

    Ok(UsageSnapshot {
        quota_groups: groups,
        token_activity,
        credits: normalize_credits(main),
        updated_at,
        stale: false,
    })
}

fn normalize_group(id: &str, value: &Value, primary: bool) -> QuotaGroup {
    let name = value
        .get("limitName")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if id.eq_ignore_ascii_case("codex") {
                "Codex".to_string()
            } else {
                id.to_string()
            }
        });

    let mut windows = Vec::new();
    for key in ["primary", "secondary"] {
        if let Some(window) = value.get(key).filter(|window| window.is_object()) {
            windows.push(normalize_window(key, window));
        }
    }

    QuotaGroup {
        id: id.to_string(),
        name,
        primary,
        plan_type: value
            .get("planType")
            .and_then(Value::as_str)
            .map(str::to_string),
        windows,
    }
}

fn normalize_window(key: &str, value: &Value) -> QuotaWindow {
    let used = value
        .get("usedPercent")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 100.0);
    let duration = value.get("windowDurationMins").and_then(Value::as_i64);

    QuotaWindow {
        key: key.to_string(),
        label: duration_label(duration),
        used_percent: used,
        remaining_percent: (100.0 - used).clamp(0.0, 100.0),
        window_duration_mins: duration,
        resets_at: value.get("resetsAt").and_then(Value::as_i64),
    }
}

fn duration_label(duration: Option<i64>) -> String {
    match duration {
        Some(300) => "5-hour allowance".to_string(),
        Some(10_080) => "Weekly allowance".to_string(),
        Some(minutes) if minutes > 0 && minutes % 1_440 == 0 => unit_label(minutes / 1_440, "day"),
        Some(minutes) if minutes > 0 && minutes % 60 == 0 => unit_label(minutes / 60, "hour"),
        Some(minutes) if minutes > 0 => unit_label(minutes, "minute"),
        _ => "Allowance".to_string(),
    }
}

fn unit_label(value: i64, unit: &str) -> String {
    format!("{value}-{unit} allowance")
}

fn normalize_token_activity(value: &Value, today: &str) -> TokenActivity {
    let summary = value.get("summary").unwrap_or(&Value::Null);
    let today_tokens = value
        .get("dailyUsageBuckets")
        .and_then(Value::as_array)
        .and_then(|buckets| {
            buckets
                .iter()
                .find(|bucket| bucket.get("startDate").and_then(Value::as_str) == Some(today))
        })
        .and_then(|bucket| decimal_string(bucket.get("tokens")))
        .or_else(|| Some("0".to_string()));

    TokenActivity {
        today_tokens,
        lifetime_tokens: decimal_string(summary.get("lifetimeTokens")),
        peak_daily_tokens: decimal_string(summary.get("peakDailyTokens")),
    }
}

fn normalize_credits(value: &Value) -> Option<CreditState> {
    let credits = value.get("credits").filter(|credits| credits.is_object());
    let individual = value
        .get("individualLimit")
        .filter(|limit| limit.is_object())
        .map(|limit| SpendControl {
            limit: string_value(limit.get("limit")).unwrap_or_default(),
            used: string_value(limit.get("used")).unwrap_or_default(),
            remaining_percent: limit
                .get("remainingPercent")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .clamp(0.0, 100.0),
            resets_at: limit.get("resetsAt").and_then(Value::as_i64).unwrap_or(0),
        });
    let reached = value
        .get("spendControlReached")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let has_credits = credits
        .and_then(|credits| credits.get("hasCredits"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let unlimited = credits
        .and_then(|credits| credits.get("unlimited"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if credits.is_none() && individual.is_none() && !reached {
        return None;
    }
    if !has_credits && !unlimited && individual.is_none() && !reached {
        return None;
    }

    Some(CreditState {
        has_credits,
        unlimited,
        balance: credits.and_then(|credits| string_value(credits.get("balance"))),
        spend_control_reached: reached,
        individual_limit: individual,
    })
}

fn decimal_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(number) => Some(number.to_string()),
        Value::String(value) if value.chars().all(|character| character.is_ascii_digit()) => {
            Some(value.clone())
        }
        _ => None,
    }
}

fn string_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_weekly_only_response_and_large_tokens() {
        let limits = json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": { "usedPercent": 4, "windowDurationMins": 10080, "resetsAt": 2000 },
                "secondary": null,
                "credits": { "hasCredits": false, "unlimited": false, "balance": "0" },
                "planType": "plus"
            },
            "rateLimitsByLimitId": null
        });
        let usage = json!({
            "summary": { "lifetimeTokens": 9007199254740993_u64, "peakDailyTokens": 42 },
            "dailyUsageBuckets": [{ "startDate": "2026-07-20", "tokens": 123456789 }]
        });

        let snapshot = normalize_snapshot(&limits, Some(&usage), "2026-07-20", 100).unwrap();
        assert_eq!(snapshot.quota_groups.len(), 1);
        assert_eq!(
            snapshot.quota_groups[0].windows[0].label,
            "Weekly allowance"
        );
        assert_eq!(snapshot.quota_groups[0].windows[0].remaining_percent, 96.0);
        assert_eq!(
            snapshot.token_activity.today_tokens.as_deref(),
            Some("123456789")
        );
        assert_eq!(
            snapshot.token_activity.lifetime_tokens.as_deref(),
            Some("9007199254740993")
        );
        assert!(snapshot.credits.is_none());
    }

    #[test]
    fn prefers_codex_bucket_and_keeps_other_buckets_expanded() {
        let codex = json!({
            "limitId": "codex",
            "limitName": null,
            "primary": { "usedPercent": 28, "windowDurationMins": 300, "resetsAt": 1000 },
            "secondary": { "usedPercent": 43, "windowDurationMins": 10080, "resetsAt": 2000 },
            "credits": { "hasCredits": true, "unlimited": false, "balance": "12.50" }
        });
        let limits = json!({
            "rateLimits": codex.clone(),
            "rateLimitsByLimitId": {
                "codex": codex,
                "spark": {
                    "limitId": "spark",
                    "limitName": "Spark",
                    "primary": { "usedPercent": 10, "windowDurationMins": 60, "resetsAt": null }
                }
            }
        });

        let snapshot = normalize_snapshot(&limits, None, "2026-07-20", 100).unwrap();
        assert_eq!(snapshot.quota_groups.len(), 2);
        assert!(snapshot.quota_groups[0].primary);
        assert_eq!(snapshot.quota_groups[0].windows.len(), 2);
        assert_eq!(snapshot.quota_groups[1].name, "Spark");
        assert_eq!(
            snapshot.quota_groups[1].windows[0].label,
            "1-hour allowance"
        );
        assert_eq!(snapshot.credits.unwrap().balance.as_deref(), Some("12.50"));
    }

    #[test]
    fn clamps_percentages_and_handles_missing_today() {
        let limits = json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": { "usedPercent": 140, "windowDurationMins": null, "resetsAt": null },
                "secondary": { "usedPercent": -10, "windowDurationMins": 90, "resetsAt": null },
                "individualLimit": { "limit": "100", "used": "75", "remainingPercent": 25, "resetsAt": 99 },
                "spendControlReached": false
            }
        });
        let usage = json!({
            "summary": { "lifetimeTokens": null },
            "dailyUsageBuckets": [{ "startDate": "2026-07-19", "tokens": 10 }]
        });

        let snapshot = normalize_snapshot(&limits, Some(&usage), "2026-07-20", 100).unwrap();
        assert_eq!(snapshot.quota_groups[0].windows[0].remaining_percent, 0.0);
        assert_eq!(snapshot.quota_groups[0].windows[1].remaining_percent, 100.0);
        assert_eq!(
            snapshot.quota_groups[0].windows[1].label,
            "90-minute allowance"
        );
        assert_eq!(snapshot.token_activity.today_tokens.as_deref(), Some("0"));
        assert!(snapshot.credits.unwrap().individual_limit.is_some());
    }
}
