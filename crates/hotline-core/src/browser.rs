//! A teammate's own browser on the desk: a web view the desktop app keeps for
//! each teammate that has no computer, so a quick look at a page needs no
//! container, MCP server or Playwright. The core owns what the agent may ask
//! of it (the `browser` tool) and which addresses it may load; the app owns
//! the web view and implements [`DeskBrowser`]. A desk with no window
//! (`hotline serve`) has the same browser in a helper process of its own
//! where WebKitGTK is installed (`helper.rs`), and none where it is not; a
//! Mac before macOS 14 ([`kept_here`]) installs none. No teammate on a desk
//! without one is offered the tool.
//!
//! What the desk and every browser share, the trait and its types, the
//! [`Gate`] and the helper's [`pipe`], is `hotline-browser-core`, re-exported
//! here whole, so the helper builds without the core; what only the core
//! needs stays here.

pub mod authenticator;
#[cfg(unix)]
pub mod helper;

pub use hotline_browser_core::*;
use url::Url;

/// The most one answer of a page's text or links may be, in characters: a
/// longer page comes in parts, each asked for by number. Kept under where
/// an agent's harness moves a long tool answer out to a file.
pub const READ_LIMIT: usize = 20_000;

/// The page's size when nothing asked for another, in CSS pixels.
pub const VIEWPORT: (u32, u32) = (1280, 800);
/// The narrowest and widest, shortest and tallest page `viewport` takes.
pub const VIEWPORT_WIDTH: std::ops::RangeInclusive<u32> = 240..=3840;
pub const VIEWPORT_HEIGHT: std::ops::RangeInclusive<u32> = 240..=2400;

/// The common sizes `viewport` takes by name, so a teammate asks for a
/// phone or a tablet without knowing any one model's: a phone's 390 by 844,
/// a tablet's 820 by 1180 (both upright), the desk's own, and a wide screen.
pub const VIEWPORT_SIZES: [(&str, Option<(u32, u32)>); 4] = [
    ("phone", Some((390, 844))),
    ("tablet", Some((820, 1180))),
    ("desktop", None),
    ("wide", Some((1920, 1080))),
];

/// A named size, as `viewport`'s `size` gives it.
pub fn viewport_named(name: &str) -> Result<Option<(u32, u32)>, String> {
    VIEWPORT_SIZES
        .iter()
        .find(|(named, _)| named.eq_ignore_ascii_case(name.trim()))
        .map(|(_, size)| *size)
        .ok_or_else(|| "viewport's size is phone, tablet, desktop or wide.".to_string())
}

/// The size `viewport` was asked for: `width`, and `height` or the default
/// height; no width is the desk's own size again. Out of range is said, not
/// clamped, so the teammate knows what it got.
pub fn viewport_of(width: Option<u64>, height: Option<u64>) -> Result<Option<(u32, u32)>, String> {
    let Some(width) = width else {
        return Ok(None);
    };
    let height = height.unwrap_or(u64::from(VIEWPORT.1));
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err("viewport takes a width and height in pixels.".into());
    };
    if !VIEWPORT_WIDTH.contains(&width) || !VIEWPORT_HEIGHT.contains(&height) {
        return Err(format!(
            "viewport takes a width from {} to {} and a height from {} to {} pixels.",
            VIEWPORT_WIDTH.start(),
            VIEWPORT_WIDTH.end(),
            VIEWPORT_HEIGHT.start(),
            VIEWPORT_HEIGHT.end()
        ));
    }
    Ok(Some((width, height)))
}

/// Whether a login stored for `sites` may be typed on `page`: same scheme
/// and port as one of its sites, on its host or a subdomain of it, as the
/// computer decides (docs/security.md). A site is `https://` unless it is
/// localhost.
pub fn login_fits(sites: &[String], page: &Url) -> bool {
    let Some(host) = page.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    sites.iter().any(|site| {
        let Ok(site) = Url::parse(site) else {
            return false;
        };
        let Some(site_host) = site.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };
        site.scheme() == page.scheme()
            && site.port_or_known_default() == page.port_or_known_default()
            && (host == site_host || host.ends_with(&format!(".{site_host}")))
    })
}

/// The six-digit code a base32 TOTP seed gives at `unix_seconds`
/// (RFC 6238: SHA-1, 30-second steps).
pub fn totp_code(seed: &str, unix_seconds: u64) -> Option<String> {
    use hmac::{Hmac, Mac};
    let cleaned: String = seed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .collect::<String>()
        .to_ascii_uppercase();
    let key = data_encoding::BASE32_NOPAD
        .decode(cleaned.as_bytes())
        .ok()?;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&key).ok()?;
    mac.update(&(unix_seconds / 30).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 0x0f);
    let value = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    Some(format!("{:06}", value % 1_000_000))
}

/// The oldest macOS whose WebKit keeps a web data store by an id
/// (`WKWebsiteDataStore dataStoreForIdentifier:`), which is what gives each
/// teammate's browser a store of its own. Below it every web view would
/// share the default store, Hotline's own window's, with each other teammate
/// and with any cookies lent to one, so no desk browser is kept there.
pub const MACOS_OWN_STORES: u32 = 14;

/// Whether this system keeps a desk browser at all: macOS from
/// [`MACOS_OWN_STORES`] on; Linux, where WebKitGTK gives each teammate a
/// web context on a data directory of its own; and Windows, where WebView2
/// gives each an environment on a data directory of its own. The app
/// installs one only where this holds, and a teammate is told it has one only
/// where it does, so the two never disagree.
pub fn kept_here() -> bool {
    #[cfg(target_os = "macos")]
    {
        static KEPT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *KEPT.get_or_init(|| kept_on_macos(macos_major()))
    }
    #[cfg(any(target_os = "linux", windows))]
    {
        true
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        false
    }
}

/// [`kept_here`] on macOS, by its major version; an unreadable one keeps
/// none.
#[cfg_attr(
    not(any(target_os = "macos", test)),
    allow(dead_code, reason = "only macOS asks its version")
)]
fn kept_on_macos(major: Option<u32>) -> bool {
    major.is_some_and(|major| major >= MACOS_OWN_STORES)
}

/// This Mac's major version, as the kernel reports the product's
/// (`kern.osproductversion`, "14.5").
#[cfg(target_os = "macos")]
fn macos_major() -> Option<u32> {
    let mut buffer = [0u8; 32];
    let mut size = buffer.len();
    // SAFETY: a NUL-terminated name, a buffer of `size` bytes and its size,
    // which the call writes back as the length it filled.
    let failed = unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            buffer.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0;
    if failed {
        return None;
    }
    let version = std::str::from_utf8(&buffer[..size.min(buffer.len())]).ok()?;
    version
        .trim_end_matches('\0')
        .split('.')
        .next()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn a_desk_browser_is_kept_only_on_macos_14_or_later() {
        assert!(!kept_on_macos(None));
        assert!(!kept_on_macos(Some(12)));
        assert!(!kept_on_macos(Some(13)));
        assert!(kept_on_macos(Some(14)));
        assert!(kept_on_macos(Some(26)));
        #[cfg(target_os = "macos")]
        {
            let major = macos_major().expect("this Mac's version");
            assert!(major >= 13, "{major}");
            assert_eq!(kept_here(), major >= MACOS_OWN_STORES);
        }
        #[cfg(any(target_os = "linux", windows))]
        assert!(kept_here());
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        assert!(!kept_here());
    }

    #[test]
    fn a_login_fits_its_own_sites_and_their_subdomains_only() {
        let sites = vec![
            "https://github.com".to_string(),
            "http://localhost:3000".to_string(),
        ];
        assert!(login_fits(&sites, &url("https://github.com/login")));
        assert!(login_fits(&sites, &url("https://gist.github.com/")));
        assert!(login_fits(&sites, &url("http://localhost:3000/signin")));
        assert!(
            !login_fits(&sites, &url("http://github.com/login")),
            "scheme"
        );
        assert!(
            !login_fits(&sites, &url("https://github.com.evil.dev/")),
            "suffix trick"
        );
        assert!(
            !login_fits(&sites, &url("https://evilgithub.com/")),
            "not a subdomain"
        );
        assert!(!login_fits(&sites, &url("http://localhost:3001/")), "port");
    }

    #[test]
    fn a_totp_code_is_rfc_6238() {
        // RFC 6238's SHA-1 test seed, "12345678901234567890", in base32.
        let seed = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        assert_eq!(totp_code(seed, 59).as_deref(), Some("287082"));
        assert_eq!(totp_code(seed, 1_111_111_109).as_deref(), Some("081804"));
        assert_eq!(totp_code("not base32!", 59), None);
    }

    #[test]
    fn a_viewport_is_a_size_in_range_or_the_desks_own() {
        assert_eq!(viewport_of(Some(390), Some(844)), Ok(Some((390, 844))));
        assert_eq!(viewport_of(Some(768), None), Ok(Some((768, VIEWPORT.1))));
        assert_eq!(viewport_of(None, Some(844)), Ok(None));
        assert!(viewport_of(Some(100), None).is_err());
        assert!(viewport_of(Some(390), Some(10_000)).is_err());
        assert_eq!(viewport_named("Phone"), Ok(Some((390, 844))));
        assert_eq!(viewport_named("desktop"), Ok(None));
        assert!(viewport_named("iPhone 15").is_err());
    }
}
