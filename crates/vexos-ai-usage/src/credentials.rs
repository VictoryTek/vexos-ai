//! Claude Code's saved sign-in: the only file in this repository that opens
//! the login file. `tests::only_this_file_names_the_login_file` keeps it so.
//!
//! What leaves this module: the access token (as a [`Secret`], for one
//! `Authorization` header), two expiry times, and a display plan label. The
//! refresh token is never read into a value, and nothing is ever written back:
//! only the Claude CLI renews the token, so a lapsed one is reported, not fixed.

use std::path::{Path, PathBuf};

use crate::secret::Secret;

/// The login file's name inside Claude's config directory.
const LOGIN_FILE: &str = ".credentials.json";

pub struct Login {
    pub token: Secret,
    /// When the access token lapses, in ms; 0 when the file does not say.
    pub expires_at_ms: u64,
    /// When the refresh token lapses, in ms; 0 when the file does not say.
    pub refresh_expires_at_ms: u64,
    /// "Pro", "Max 5x", "Team 5x", or "" when unknown.
    pub plan: String,
}

/// There is no usable sign-in: no file, no `claudeAiOauth` object, or an empty
/// token. The caller reports "waiting for auth" and nothing more.
#[derive(Debug, PartialEq, Eq)]
pub struct NoLogin;

/// Claude's config directory: `CLAUDE_CONFIG_DIR` when set and not empty, else
/// `~/.claude`.
pub fn config_dir(home: &Path, configured: Option<&str>) -> PathBuf {
    match configured {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home.join(".claude"),
    }
}

/// The CLI's profile for a config directory: a legacy `.config.json` inside it,
/// else `~/.claude.json` for the default directory, else `<dir>/.claude.json`.
/// A copy anywhere else is never refreshed, and describes another account.
fn profile_path(home: &Path, dir: &Path, configured: Option<&str>) -> PathBuf {
    let legacy = dir.join(".config.json");
    if legacy.exists() {
        return legacy;
    }
    let default_dir = home.join(".claude");
    let pointed_at_default =
        configured.is_some_and(|c| !c.is_empty() && Path::new(c) == default_dir);
    if dir == default_dir && (configured.is_none_or(str::is_empty) || pointed_at_default) {
        return home.join(".claude.json");
    }
    dir.join(".claude.json")
}

fn read_object(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.is_object().then_some(value)
}

fn text_of(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned()
}

fn millis_of(value: &serde_json::Value, key: &str) -> u64 {
    let Some(n) = value.get(key) else { return 0 };
    if let Some(n) = n.as_u64() {
        return n;
    }
    n.as_f64()
        .filter(|f| f.is_finite() && *f > 0.0)
        .map_or(0, |f| f as u64)
}

/// The multiplier in a tier such as `default_claude_max_5x`: "5x".
fn max_multiplier(tier: &str) -> Option<String> {
    let lower = tier.to_ascii_lowercase();
    let at = lower.find("max_")? + 4;
    let rest = &lower[at..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || !rest[digits.len()..].starts_with('x') {
        return None;
    }
    Some(format!("{digits}x"))
}

/// The rate-limit tier names the ceiling, not the plan: a Team premium seat
/// runs on `default_claude_max_5x` while paying for Team. Lead with the
/// subscription and keep the multiplier as its qualifier, so Max reads
/// "Max 5x" and a Team seat reads "Team 5x" rather than passing as Max.
pub fn plan_label(tier: &str, subscription: &str) -> String {
    let mut chars = subscription.chars();
    let plan: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    match max_multiplier(tier) {
        Some(multiplier) if plan.is_empty() => format!("Max {multiplier}"),
        Some(multiplier) => format!("{plan} {multiplier}"),
        None => plan,
    }
}

/// Only characters a plan name can contain, so a hostile file cannot put
/// anything else on screen.
fn clean_plan(plan: String) -> String {
    plan.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ')
        .take(24)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The tier the CLI's profile states. An upgrade does not restamp the tier in
/// the login file (a refresh keeps the old value), so the profile, which is
/// refreshed against the account, is asked first.
fn profile_tier(path: &Path) -> String {
    let Some(profile) = read_object(path) else {
        return String::new();
    };
    let Some(account) = profile.get("oauthAccount") else {
        return String::new();
    };
    for key in ["userRateLimitTier", "organizationRateLimitTier"] {
        let tier = text_of(account, key);
        if !tier.is_empty() {
            return tier;
        }
    }
    String::new()
}

/// Reads the sign-in. See the module comment for what leaves.
pub fn read_login(home: &Path, configured: Option<&str>) -> Result<Login, NoLogin> {
    let dir = config_dir(home, configured);
    let file = read_object(&dir.join(LOGIN_FILE)).ok_or(NoLogin)?;
    let oauth = file
        .get("claudeAiOauth")
        .filter(|v| v.is_object())
        .ok_or(NoLogin)?;
    let token = Secret::new(text_of(oauth, "accessToken"));
    if token.is_empty() {
        return Err(NoLogin);
    }
    let mut tier = profile_tier(&profile_path(home, &dir, configured));
    if max_multiplier(&tier).is_none() {
        tier = text_of(oauth, "rateLimitTier");
    }
    Ok(Login {
        token,
        expires_at_ms: millis_of(oauth, "expiresAt"),
        refresh_expires_at_ms: millis_of(oauth, "refreshTokenExpiresAt"),
        plan: clean_plan(plan_label(&tier, &text_of(oauth, "subscriptionType"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vexos-usage-cred-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    fn login_file(home: &Path, body: &str) {
        fs::write(home.join(".claude").join(LOGIN_FILE), body).unwrap();
    }

    #[test]
    fn reads_token_expiries_and_plan() {
        let home = scratch("ok");
        login_file(
            &home,
            r#"{"claudeAiOauth":{"accessToken":"tok","refreshToken":"ref","expiresAt":1000,
               "refreshTokenExpiresAt":2000.0,"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}"#,
        );
        let login = read_login(&home, None).unwrap();
        assert_eq!(login.token.expose(), "tok");
        assert_eq!(
            (login.expires_at_ms, login.refresh_expires_at_ms),
            (1000, 2000)
        );
        assert_eq!(login.plan, "Pro");
    }

    #[test]
    fn missing_or_empty_is_no_login() {
        let home = scratch("none");
        assert!(read_login(&home, None).is_err());
        login_file(&home, "not json");
        assert!(read_login(&home, None).is_err());
        login_file(&home, r#"{"other":1}"#);
        assert!(read_login(&home, None).is_err());
        login_file(&home, r#"{"claudeAiOauth":{"accessToken":""}}"#);
        assert!(read_login(&home, None).is_err());
        login_file(&home, r#"{"claudeAiOauth":"nope"}"#);
        assert!(read_login(&home, None).is_err());
    }

    #[test]
    fn plan_labels() {
        assert_eq!(plan_label("default_claude_max_5x", "max"), "Max 5x");
        assert_eq!(plan_label("default_claude_max_20x", "max"), "Max 20x");
        assert_eq!(plan_label("default_claude_max_5x", "team"), "Team 5x");
        assert_eq!(plan_label("", "pro"), "Pro");
        assert_eq!(plan_label("default_claude_max_5x", ""), "Max 5x");
        assert_eq!(plan_label("default_claude_pro", ""), "");
        assert_eq!(plan_label("max_x", "max"), "Max");
    }

    #[test]
    fn the_profile_tier_outranks_a_stale_login_tier() {
        let home = scratch("profile");
        login_file(
            &home,
            r#"{"claudeAiOauth":{"accessToken":"t","subscriptionType":"max","rateLimitTier":"default_claude_max_5x"}}"#,
        );
        fs::write(
            home.join(".claude.json"),
            r#"{"oauthAccount":{"userRateLimitTier":"default_claude_max_20x"}}"#,
        )
        .unwrap();
        assert_eq!(read_login(&home, None).unwrap().plan, "Max 20x");
    }

    #[test]
    fn the_config_dir_override_is_honoured() {
        let home = scratch("override");
        let other = home.join("elsewhere");
        fs::create_dir_all(&other).unwrap();
        fs::write(
            other.join(LOGIN_FILE),
            r#"{"claudeAiOauth":{"accessToken":"other","subscriptionType":"pro"}}"#,
        )
        .unwrap();
        let login = read_login(&home, other.to_str()).unwrap();
        assert_eq!(login.token.expose(), "other");
        // An empty override means the default.
        login_file(&home, r#"{"claudeAiOauth":{"accessToken":"default"}}"#);
        assert_eq!(
            read_login(&home, Some("")).unwrap().token.expose(),
            "default"
        );
    }

    #[test]
    fn a_hostile_plan_name_is_cleaned() {
        let home = scratch("hostile");
        login_file(
            &home,
            r#"{"claudeAiOauth":{"accessToken":"t","subscriptionType":"pro\u0000<b>\n&x"}}"#,
        );
        assert_eq!(read_login(&home, None).unwrap().plan, "Probx");
    }

    #[test]
    fn only_this_file_names_the_login_file() {
        // The login file's name, built so that this test does not name it.
        let name = format!(".{}.json", "credentials");
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        for entry in fs::read_dir(&src).unwrap() {
            let path = entry.unwrap().path();
            let text = fs::read_to_string(&path).unwrap();
            if text.contains(&name) {
                found.push(path.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
        found.sort();
        assert_eq!(found, vec!["credentials.rs".to_owned()]);
    }
}
