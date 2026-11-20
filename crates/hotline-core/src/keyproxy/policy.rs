//! The key-minting endpoints a protected key is never sent to.
//!
//! A key that may mint keys would hand a job a fresh, reusable one in the
//! reply, which no stand-in covers. The desk sends each service's routes in
//! `use.refuse` (from `services.json`); a request carrying a value to one is
//! answered here and never sent. The built-in list below is the minimum for
//! the services Hotline names, refused whatever the desk sent: its list adds
//! to this one and never replaces it. An "Other" origin is on none of it.

/// Origin, then path segments; `*` is any one segment, and the path may go
/// on below the last.
const REFUSED: &[(&str, &[&str])] = &[
    // OAuth Authorizations API: creates and lists personal tokens.
    ("https://api.github.com", &["authorizations"]),
    // A GitHub App's installation token.
    (
        "https://api.github.com",
        &["app", "installations", "*", "access_tokens"],
    ),
    // OpenAI admin keys, project keys, and service accounts (which come with
    // a key).
    (
        "https://api.openai.com",
        &["v1", "organization", "admin_api_keys"],
    ),
    (
        "https://api.openai.com",
        &["v1", "organization", "projects", "*", "api_keys"],
    ),
    (
        "https://api.openai.com",
        &["v1", "organization", "projects", "*", "service_accounts"],
    ),
    // Anthropic admin key routes.
    (
        "https://api.anthropic.com",
        &["v1", "organizations", "api_keys"],
    ),
];

const MAX_ROUTES: usize = 64;
const MAX_ROUTE_BYTES: usize = 256;

/// The desk's `refuse` list as patterns, or why it is not one. A route is a
/// path such as `/app/installations/*/access_tokens`.
pub(super) fn parse(routes: &[String]) -> Result<Vec<Vec<String>>, String> {
    if routes.len() > MAX_ROUTES {
        return Err(format!("at most {MAX_ROUTES} refused routes"));
    }
    routes
        .iter()
        .map(|route| {
            let ok = route.starts_with('/')
                && route.len() <= MAX_ROUTE_BYTES
                && route
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && !b"?#%\\".contains(&b));
            let pattern: Vec<String> = route
                .split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_ascii_lowercase)
                .collect();
            if !ok || pattern.is_empty() || pattern.iter().any(|s| s == "." || s == "..") {
                return Err(format!("{route:?} is not a route to refuse"));
            }
            Ok(pattern)
        })
        .collect()
}

/// Whether `path` (the request target's path, query excluded) is one of
/// `patterns`.
pub(super) fn listed(patterns: &[Vec<String>], path: &str) -> bool {
    let segments = segments(path);
    patterns.iter().any(|pattern| hit(pattern, &segments))
}

/// The built-in list: whether `path` on `origin` is refused for every key,
/// whatever the desk sent.
pub(super) fn refused(origin: &str, path: &str) -> bool {
    let segments = segments(path);
    REFUSED
        .iter()
        .any(|(o, pattern)| *o == origin && hit(pattern, &segments))
}

fn hit<S: AsRef<str>>(pattern: &[S], segments: &[Vec<u8>]) -> bool {
    segments.len() >= pattern.len()
        && pattern.iter().zip(segments).all(|(want, got)| {
            let want = want.as_ref();
            want == "*" || matches(want, got)
        })
}

/// A segment matches a name in any case, with `;params` or `.format` after it.
fn matches(want: &str, got: &[u8]) -> bool {
    let want = want.as_bytes();
    got.len() >= want.len()
        && got[..want.len()].eq_ignore_ascii_case(want)
        && matches!(got.get(want.len()), None | Some(b'.' | b';'))
}

/// The path as a server would most likely read it: percent-decoded (an
/// encoded slash splits too), empty and `.` segments dropped, `..` applied.
fn segments(path: &str) -> Vec<Vec<u8>> {
    let decoded = decode(path.as_bytes());
    let mut out: Vec<Vec<u8>> = Vec::new();
    for segment in decoded.split(|&b| b == b'/' || b == b'\\') {
        match segment {
            b"" | b"." => {}
            b".." => {
                out.pop();
            }
            _ => out.push(segment.to_vec()),
        }
    }
    out
}

fn decode(bytes: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{listed, parse, refused};

    #[test]
    fn the_desks_routes_are_honoured() {
        let routes = parse(&["/app/installations/*/access_tokens".to_owned()]).unwrap();
        assert!(listed(&routes, "/app/installations/9/access_tokens"));
        assert!(listed(&routes, "/APP//installations/9/access_tokens.json"));
        assert!(!listed(&routes, "/authorizations"));
        for bad in ["authorizations", "/", "/a?b", "/a/../b", "/a%2fb"] {
            assert!(parse(&[bad.to_owned()]).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_minting_routes_are_refused_in_every_spelling() {
        let gh = "https://api.github.com";
        for path in [
            "/authorizations",
            "/authorizations/",
            "/authorizations/12",
            "//authorizations",
            "/AUTHORIZATIONS",
            "/%61uthorizations",
            "/x/../authorizations",
            "/authorizations.json",
            "/authorizations;x",
            "/app/installations/77/access_tokens",
            "/app%2Finstallations/77/access_tokens",
        ] {
            assert!(refused(gh, path), "{path} should be refused");
        }
        assert!(refused(
            "https://api.openai.com",
            "/v1/organization/admin_api_keys"
        ));
        assert!(refused(
            "https://api.openai.com",
            "/v1/organization/projects/p1/service_accounts"
        ));
        assert!(refused(
            "https://api.anthropic.com",
            "/v1/organizations/api_keys/k1"
        ));
    }

    #[test]
    fn ordinary_routes_and_other_origins_pass() {
        let gh = "https://api.github.com";
        for path in [
            "/user",
            "/repos/o/r",
            "/authorizationsx",
            "/app/installations",
        ] {
            assert!(!refused(gh, path), "{path} should pass");
        }
        assert!(!refused("https://api.example.test", "/authorizations"));
        assert!(!refused("https://api.openai.com", "/v1/chat/completions"));
    }
}
