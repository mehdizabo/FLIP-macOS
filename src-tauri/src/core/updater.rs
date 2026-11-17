use crate::core::logging::log_event;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::process::{Command, Stdio};

pub const DEFAULT_GITHUB_RELEASES_API: &str =
    "https://api.github.com/repos/thetinyfoundry/FLIP/releases/latest";
pub const DEFAULT_RELEASES_PAGE_URL: &str = "https://github.com/thetinyfoundry/FLIP/releases";
pub const CURL_PATH: &str = "/usr/bin/curl";
pub const UPDATE_TIMEOUT_SECS: u64 = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_tag: String,
    pub release_name: String,
    pub html_url: String,
    pub published_at: Option<String>,
    pub body: String,
    pub update_available: bool,
    pub checked_at_utc: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn parse_core_version(core: &str) -> Option<[u64; 4]> {
    let parts: Vec<&str> = core.split('.').collect();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    let mut nums = [0u64; 4];
    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        nums[idx] = part.parse::<u64>().ok()?;
    }
    Some(nums)
}

fn compare_prerelease(current_pre: &str, remote_pre: &str) -> Ordering {
    let c_parts: Vec<&str> = current_pre.split('.').collect();
    let r_parts: Vec<&str> = remote_pre.split('.').collect();
    let shared = c_parts.len().min(r_parts.len());

    for i in 0..shared {
        let cs = c_parts[i];
        let rs = r_parts[i];
        let c_num = !cs.is_empty() && cs.bytes().all(|b| b.is_ascii_digit());
        let r_num = !rs.is_empty() && rs.bytes().all(|b| b.is_ascii_digit());

        let ord = match (c_num, r_num) {
            (true, true) => match (cs.parse::<u128>(), rs.parse::<u128>()) {
                (Ok(cn), Ok(rn)) => rn.cmp(&cn),
                _ => rs.cmp(cs),
            },
            // SemVer 2.0.0: numeric identifiers always have lower precedence than non-numeric
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => rs.to_ascii_lowercase().cmp(&cs.to_ascii_lowercase()),
        };

        if ord != Ordering::Equal {
            return ord;
        }
    }

    r_parts.len().cmp(&c_parts.len())
}

/// Ported from FLIP-windows `UpdateChecker.IsRemoteNewer` with SemVer + prerelease handling.
pub fn is_remote_newer(current_raw: &str, remote_tag_raw: &str) -> bool {
    let c_full = current_raw
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('+')
        .next()
        .unwrap_or("");
    let r_full = remote_tag_raw
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('+')
        .next()
        .unwrap_or("");

    if r_full.is_empty() || c_full.eq_ignore_ascii_case(r_full) {
        return false;
    }

    let (c_core, c_pre) = match c_full.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre)),
        Some((core, _)) => (core, None),
        None => (c_full, None),
    };
    let (r_core, r_pre) = match r_full.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre)),
        Some((core, _)) => (core, None),
        None => (r_full, None),
    };

    let (Some(v_cur), Some(v_rem)) = (parse_core_version(c_core), parse_core_version(r_core)) else {
        return false;
    };

    if v_rem != v_cur {
        return v_rem > v_cur;
    }

    match (c_pre, r_pre) {
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => false,
        (Some(cp), Some(rp)) => compare_prerelease(cp, rp) == Ordering::Greater,
    }
}

pub fn normalize_github_releases_api_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(DEFAULT_GITHUB_RELEASES_API.to_string());
    }
    if trimmed.starts_with('-') || trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Invalid update check URL.".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("https://") {
        return Err("Update check URL must use https://.".to_string());
    }

    let gh_rest = lower
        .strip_prefix("https://github.com/")
        .map(|_| &trimmed["https://github.com/".len()..])
        .or_else(|| {
            lower
                .strip_prefix("https://www.github.com/")
                .map(|_| &trimmed["https://www.github.com/".len()..])
        });

    if let Some(rest) = gh_rest {
        let without_query = rest.split(['?', '#']).next().unwrap_or(rest);
        let clean = without_query.trim_end_matches('/');
        let mut segments = clean.split('/').filter(|s| !s.is_empty());
        let owner = segments
            .next()
            .ok_or_else(|| "GitHub repository URL is missing owner.".to_string())?;
        let raw_repo = segments
            .next()
            .ok_or_else(|| "GitHub repository URL is missing repository name.".to_string())?;
        let repo = raw_repo.strip_suffix(".git").unwrap_or(raw_repo);
        if repo.is_empty() {
            return Err("GitHub repository URL is missing repository name.".to_string());
        }
        return Ok(format!(
            "https://api.github.com/repos/{owner}/{repo}/releases/latest"
        ));
    }

    if lower.starts_with("https://api.github.com/repos/") {
        let rest = &trimmed["https://api.github.com/repos/".len()..];
        let without_query = rest.split(['?', '#']).next().unwrap_or(rest);
        let clean = without_query.trim_end_matches('/');
        let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() == 2 {
            let repo = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
            return Ok(format!(
                "https://api.github.com/repos/{}/{repo}/releases/latest",
                parts[0]
            ));
        }
        return Ok(format!("https://api.github.com/repos/{rest}"));
    }

    Ok(trimmed.to_string())
}

pub fn validate_release_html_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('-')
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("Invalid GitHub release URL.".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("https://github.com/") {
        return Err("Release URL must start with https://github.com/.".to_string());
    }
    Ok(trimmed.to_string())
}

pub fn sanitize_release_html_url(raw: Option<&str>, fallback: &str) -> String {
    if let Some(candidate) = raw {
        if let Ok(valid) = validate_release_html_url(candidate) {
            return valid;
        }
    }
    fallback.to_string()
}

fn truncate_release_body(raw: &str, max_chars: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn parse_github_release_json(
    raw_json: &str,
    current_ver: &str,
) -> Result<UpdateCheckResult, String> {
    let val: Value = serde_json::from_str(raw_json)
        .map_err(|e| format!("Invalid GitHub release response: {e}"))?;

    let item = if let Some(arr) = val.as_array() {
        arr.first()
            .ok_or_else(|| "No published GitHub releases found.".to_string())?
    } else if val.is_object() {
        &val
    } else {
        return Err("Unexpected GitHub release response format.".to_string());
    };

    let tag_name = item
        .get("tag_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "GitHub release response is missing `tag_name`.".to_string())?
        .to_string();

    let release_name = item
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&tag_name)
        .to_string();

    let html_url = sanitize_release_html_url(
        item.get("html_url").and_then(Value::as_str),
        DEFAULT_RELEASES_PAGE_URL,
    );

    let published_at = item
        .get("published_at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let body = item
        .get("body")
        .and_then(Value::as_str)
        .map(|s| truncate_release_body(s, 4000))
        .unwrap_or_default();

    let update_available = is_remote_newer(current_ver, &tag_name);

    Ok(UpdateCheckResult {
        current_version: current_ver.to_string(),
        latest_tag: tag_name,
        release_name,
        html_url,
        published_at,
        body,
        update_available,
        checked_at_utc: Utc::now().to_rfc3339(),
    })
}

pub fn parse_github_tags_json(
    raw_json: &str,
    current_ver: &str,
    releases_page_base: &str,
) -> Result<UpdateCheckResult, String> {
    let val: Value = serde_json::from_str(raw_json)
        .map_err(|e| format!("Invalid GitHub tags response: {e}"))?;
    let arr = val
        .as_array()
        .ok_or_else(|| "Unexpected GitHub tags response format.".to_string())?;
    let first = arr
        .first()
        .ok_or_else(|| "No published releases or tags found on GitHub.".to_string())?;
    let tag_name = first
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "GitHub tag entry is missing `name`.".to_string())?
        .to_string();

    let candidate_url = format!("{}/tag/{tag_name}", releases_page_base.trim_end_matches('/'));
    let html_url = sanitize_release_html_url(Some(&candidate_url), DEFAULT_RELEASES_PAGE_URL);
    let update_available = is_remote_newer(current_ver, &tag_name);

    Ok(UpdateCheckResult {
        current_version: current_ver.to_string(),
        latest_tag: tag_name.clone(),
        release_name: tag_name,
        html_url,
        published_at: None,
        body: String::new(),
        update_available,
        checked_at_utc: Utc::now().to_rfc3339(),
    })
}

pub fn is_curl_http_error(code: Option<i32>, stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    code == Some(22) || (code == Some(56) && lower.contains("returned error"))
}

pub fn is_curl_http_404(code: Option<i32>, stderr: &str) -> bool {
    is_curl_http_error(code, stderr) && stderr.contains("404")
}

fn format_curl_error(code: Option<i32>, stderr: &str) -> String {
    let lower = stderr.to_ascii_lowercase();
    if is_curl_http_error(code, stderr) {
        if lower.contains("403") || lower.contains("429") {
            return "GitHub API rate limit reached. Please try again in a few minutes.".to_string();
        } else if lower.contains("404") {
            return "No published GitHub release was found for this repository.".to_string();
        } else {
            return "GitHub returned an HTTP error while checking for updates.".to_string();
        }
    }
    match code {
        Some(6) | Some(7) | Some(35) | Some(52) | Some(56) => {
            "Unable to check for updates while offline or unable to reach GitHub.".to_string()
        }
        Some(28) => format!(
            "Update check timed out after {UPDATE_TIMEOUT_SECS} seconds. Check your internet connection."
        ),
        _ => {
            if lower.contains("could not resolve host") || lower.contains("failed to connect") {
                "Unable to check for updates while offline or unable to reach GitHub.".to_string()
            } else {
                format!(
                    "Unable to check for updates (network error{}).",
                    code.map(|c| format!(" #{c}")).unwrap_or_default()
                )
            }
        }
    }
}

fn fetch_with_curl(url: &str) -> Result<String, (Option<i32>, String)> {
    let timeout_str = UPDATE_TIMEOUT_SECS.to_string();
    let user_agent = format!("FLIP-macOS/{}", current_version());
    let output = Command::new(CURL_PATH)
        .args([
            "-fsSL",
            "--max-time",
            &timeout_str,
            "--proto",
            "=https",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            &format!("User-Agent: {user_agent}"),
            url,
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| (None, format!("Failed to invoke {CURL_PATH}: {e}")))?;

    if output.status.success() {
        String::from_utf8(output.stdout)
            .map_err(|e| (None, format!("Invalid UTF-8 response from GitHub: {e}")))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err((output.status.code(), stderr))
    }
}

pub fn check_github_updates(repo_url: Option<&str>) -> Result<UpdateCheckResult, String> {
    let env_override = std::env::var("FLIP_UPDATE_REPO_URL").ok();
    let effective_raw = repo_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| env_override.as_deref().map(str::trim).filter(|s| !s.is_empty()));

    let api_url = match effective_raw {
        Some(raw) => normalize_github_releases_api_url(raw)?,
        None => DEFAULT_GITHUB_RELEASES_API.to_string(),
    };

    let current = current_version();
    match fetch_with_curl(&api_url) {
        Ok(body) => {
            let parsed = parse_github_release_json(&body, current)?;
            log_event(
                "updater.checked",
                Some(json!({
                    "current": parsed.current_version,
                    "latestTag": parsed.latest_tag,
                    "updateAvailable": parsed.update_available,
                })),
            );
            Ok(parsed)
        }
        Err((code, stderr)) if is_curl_http_404(code, &stderr) && api_url.ends_with("/releases/latest") => {
            // Fallback to tags?per_page=1 if repository has tags without formal GitHub Release objects
            let tags_url = format!(
                "{}/tags?per_page=1",
                api_url.trim_end_matches("/releases/latest")
            );
            let releases_page_base = if api_url
                .to_ascii_lowercase()
                .starts_with("https://api.github.com/repos/")
            {
                let rest = &api_url["https://api.github.com/repos/".len()..];
                let slug = rest.trim_end_matches("/releases/latest");
                format!("https://github.com/{slug}/releases")
            } else {
                DEFAULT_RELEASES_PAGE_URL.to_string()
            };
            match fetch_with_curl(&tags_url) {
                Ok(tags_body) => {
                    let parsed = parse_github_tags_json(&tags_body, current, &releases_page_base)?;
                    log_event(
                        "updater.checked_tags",
                        Some(json!({
                            "current": parsed.current_version,
                            "latestTag": parsed.latest_tag,
                            "updateAvailable": parsed.update_available,
                        })),
                    );
                    Ok(parsed)
                }
                Err((code, err_str)) => {
                    let msg = format_curl_error(code, &err_str);
                    log_event("updater.error", Some(json!({ "error": &msg })));
                    Err(msg)
                }
            }
        }
        Err((code, stderr)) => {
            let msg = format_curl_error(code, &stderr);
            log_event("updater.error", Some(json!({ "error": &msg })));
            Err(msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_and_prerelease_comparison_matches_windows_input_tests() {
        // Direct ports from FLIP-windows InputTests.cs line 103
        assert!(is_remote_newer("0.4.0-beta.3", "v0.5.0"));
        assert!(is_remote_newer("0.4.0-beta.3", "v0.4.0"));
        assert!(!is_remote_newer("0.4.0-beta.3", "v0.4.0-beta.3"));

        // Additional SemVer + prerelease edge cases
        assert!(!is_remote_newer("0.4.0", "v0.4.0-beta.3"));
        assert!(!is_remote_newer("0.4.0", "v0.3.9"));
        assert!(!is_remote_newer("0.4.0", "V0.4.0"));
        assert!(!is_remote_newer("0.4.0-beta.3", "V0.4.0-BETA.3"));
        assert!(is_remote_newer("0.4.0-beta.2", "v0.4.0-beta.10"));
        assert!(!is_remote_newer("0.4.0-beta.10", "v0.4.0-beta.2"));
        assert!(is_remote_newer("0.4.0-alpha.9", "v0.4.0-beta.1"));
        assert!(!is_remote_newer("0.4.0-rc.1", "v0.4.0-beta.9"));
        assert!(is_remote_newer("0.4.0-beta", "v0.4.0-beta.1"));
        assert!(is_remote_newer("0.1.0", "v0.1.1"));
        assert!(is_remote_newer("0.1.0", "v0.2.0-rc.1"));
        assert!(is_remote_newer("0.1.0+build12", "v0.1.1+build99"));
        assert!(!is_remote_newer("0.1.0+build12", "v0.1.0+build99"));
        assert!(!is_remote_newer("0.1.0", ""));
        assert!(!is_remote_newer("0.1.0", "   "));
        assert!(!is_remote_newer("0.1.0", "not-a-version"));
        assert!(!is_remote_newer("bad", "v1.0.0"));
        assert!(is_remote_newer("0.5.0", "v0.5.0.1"));
    }

    #[test]
    fn github_url_normalization_and_html_url_sanitization() {
        assert_eq!(
            normalize_github_releases_api_url("").unwrap(),
            DEFAULT_GITHUB_RELEASES_API
        );
        assert_eq!(
            normalize_github_releases_api_url("https://github.com/thetinyfoundry/FLIP").unwrap(),
            "https://api.github.com/repos/thetinyfoundry/FLIP/releases/latest"
        );
        assert_eq!(
            normalize_github_releases_api_url("https://github.com/acme/tool.git/").unwrap(),
            "https://api.github.com/repos/acme/tool/releases/latest"
        );
        assert_eq!(
            normalize_github_releases_api_url("HTTPS://GitHub.com/acme/tool.git?tab=readme#top")
                .unwrap(),
            "https://api.github.com/repos/acme/tool/releases/latest"
        );
        assert_eq!(
            normalize_github_releases_api_url("https://api.github.com/repos/acme/tool").unwrap(),
            "https://api.github.com/repos/acme/tool/releases/latest"
        );
        assert!(normalize_github_releases_api_url("http://api.github.com/repos/a/b").is_err());
        assert!(normalize_github_releases_api_url("file:///etc/passwd").is_err());
        assert!(normalize_github_releases_api_url("--insecure").is_err());

        assert_eq!(
            validate_release_html_url("https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1")
                .unwrap(),
            "https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1"
        );
        assert!(validate_release_html_url("https://evil.example/payload").is_err());
        assert!(validate_release_html_url("javascript:alert(1)").is_err());
        assert!(validate_release_html_url("file:///Applications/Safari.app").is_err());

        assert_eq!(
            sanitize_release_html_url(
                Some("https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1"),
                DEFAULT_RELEASES_PAGE_URL
            ),
            "https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1"
        );
        assert_eq!(
            sanitize_release_html_url(Some("javascript:alert(1)"), DEFAULT_RELEASES_PAGE_URL),
            DEFAULT_RELEASES_PAGE_URL
        );
        assert_eq!(
            sanitize_release_html_url(Some("https://evil.example/payload"), DEFAULT_RELEASES_PAGE_URL),
            DEFAULT_RELEASES_PAGE_URL
        );
    }

    #[test]
    fn github_release_and_tag_json_parsing_works_and_rejects_malformed() {
        let release_json = r#"{
            "tag_name": "v0.5.1",
            "name": "FLIP v0.5.1",
            "html_url": "https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1",
            "published_at": "2026-10-24T12:00:00Z",
            "body": "Bug fixes and improvements."
        }"#;
        let parsed = parse_github_release_json(release_json, "0.1.0").unwrap();
        assert_eq!(parsed.current_version, "0.1.0");
        assert_eq!(parsed.latest_tag, "v0.5.1");
        assert_eq!(parsed.release_name, "FLIP v0.5.1");
        assert_eq!(
            parsed.html_url,
            "https://github.com/thetinyfoundry/FLIP/releases/tag/v0.5.1"
        );
        assert_eq!(
            parsed.published_at.as_deref(),
            Some("2026-10-24T12:00:00Z")
        );
        assert_eq!(parsed.body, "Bug fixes and improvements.");
        assert!(parsed.update_available);

        let same_ver = parse_github_release_json(release_json, "0.5.1").unwrap();
        assert!(!same_ver.update_available);

        let tags_json = r#"[{"name": "v0.2.0"}]"#;
        let tag_parsed = parse_github_tags_json(
            tags_json,
            "0.1.0",
            "https://github.com/thetinyfoundry/FLIP/releases",
        )
        .unwrap();
        assert_eq!(tag_parsed.latest_tag, "v0.2.0");
        assert_eq!(
            tag_parsed.html_url,
            "https://github.com/thetinyfoundry/FLIP/releases/tag/v0.2.0"
        );
        assert!(tag_parsed.update_available);

        assert!(parse_github_release_json("{}", "0.1.0").is_err());
        assert!(parse_github_release_json("not json", "0.1.0").is_err());
    }

    #[test]
    fn curl_error_messages_are_clean_and_user_friendly() {
        let offline = format_curl_error(Some(6), "curl: (6) Could not resolve host: api.github.com");
        assert!(offline.contains("offline"));
        let recv_drop = format_curl_error(Some(56), "curl: (56) LibreSSL SSL_read: Connection reset by peer");
        assert!(recv_drop.contains("offline"));
        let timeout = format_curl_error(Some(28), "Operation timed out after 8000 milliseconds");
        assert!(timeout.contains("timed out"));
        let rate = format_curl_error(Some(22), "The requested URL returned error: 403");
        assert!(rate.contains("rate limit"));
        let rate_h2 = format_curl_error(Some(56), "curl: (56) The requested URL returned error: 429");
        assert!(rate_h2.contains("rate limit"));
        let not_found_h2 = format_curl_error(Some(56), "curl: (56) The requested URL returned error: 404");
        assert!(not_found_h2.contains("No published GitHub release"));
        assert!(is_curl_http_404(Some(56), "curl: (56) The requested URL returned error: 404"));
        assert!(is_curl_http_404(Some(22), "curl: (22) The requested URL returned error: 404"));
        assert!(!is_curl_http_404(Some(56), "curl: (56) Connection reset by peer"));
    }

    #[test]
    #[ignore = "hits live github.com over network"]
    fn live_github_update_check_and_tags_fallback() {
        let res = check_github_updates(None).expect("live release check should succeed");
        assert_eq!(res.current_version, current_version());
        assert!(!res.latest_tag.is_empty());
        assert!(res.html_url.starts_with("https://github.com/"));

        // git/git uses tags without GitHub Releases, exercising the 404 -> /tags?per_page=1 fallback
        let tag_res = check_github_updates(Some("https://github.com/git/git"))
            .expect("tags fallback should succeed for repo without releases");
        assert!(!tag_res.latest_tag.is_empty());
        assert!(tag_res
            .html_url
            .starts_with("https://github.com/git/git/releases/tag/"));

        let offline_err = check_github_updates(Some("https://nonexistent.invalid/owner/repo"))
            .expect_err("invalid domain must fail cleanly");
        assert!(
            offline_err.contains("offline") || offline_err.contains("unable to reach"),
            "unexpected error: {offline_err}"
        );
    }
}
