//! Cookies the person lent a teammate on a cookie offer, put into the
//! teammate's own web storage, and taken back on Remove. The core read them
//! from the person's browser and hands them here; they go straight to
//! WebKit's cookie store and are never logged, shown or answered back, so
//! nothing here derives `Debug` on a value. The store is the open page's,
//! or, with the browser closed, the same store found by its id.

use super::{Native, SCRIPT_WAIT};
use serde_json::Value;
use tokio::sync::oneshot;

/// One lent cookie, as the core reads it from a browser.
pub(super) struct Cookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    /// Seconds since 1970; none for a session cookie.
    expires: Option<f64>,
    secure: bool,
    http_only: bool,
    /// `Lax` or `Strict`; `None` and absent send it everywhere.
    same_site: Option<String>,
}

impl Cookie {
    fn of(cookie: &Value) -> Option<Self> {
        let text = |key: &str| cookie.get(key).and_then(Value::as_str);
        let flag = |key: &str| cookie.get(key).and_then(Value::as_bool) == Some(true);
        Some(Self {
            name: text("name")?.to_string(),
            value: text("value")?.to_string(),
            domain: text("domain")
                .filter(|domain| !domain.is_empty())?
                .to_string(),
            path: text("path").unwrap_or("/").to_string(),
            expires: cookie
                .get("expires")
                .and_then(Value::as_f64)
                .filter(|expires| *expires > 0.0),
            secure: flag("secure"),
            http_only: flag("httpOnly"),
            same_site: text("sameSite").map(str::to_string),
        })
    }
}

/// Whether a cookie's domain is `site` or under it: `.github.com`,
/// `github.com` and `gist.github.com` are on `github.com`.
pub(super) fn on_site(domain: &str, site: &str) -> bool {
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    domain == site || domain.ends_with(&format!(".{site}"))
}

type Reply = oneshot::Sender<Result<(), String>>;

impl Native {
    /// Sets the lent cookies in the teammate's store.
    pub(super) async fn lend(&self, persona_id: &str, cookies: &[Value]) -> Result<(), String> {
        let cookies: Vec<Cookie> = cookies.iter().filter_map(Cookie::of).collect();
        if cookies.is_empty() {
            return Err("None of the lent cookies could be read.".into());
        }
        let (reply, answer) = oneshot::channel();
        self.in_store(persona_id, move |store| apple::set(store, &cookies, reply))?;
        Self::answered(answer).await
    }

    /// Deletes every cookie on `site` from the teammate's store.
    pub(super) async fn forget(&self, persona_id: &str, site: &str) -> Result<(), String> {
        let site = site.to_ascii_lowercase();
        let (reply, answer) = oneshot::channel();
        self.in_store(persona_id, move |store| apple::delete(store, site, reply))?;
        Self::answered(answer).await
    }

    async fn answered(answer: oneshot::Receiver<Result<(), String>>) -> Result<(), String> {
        tokio::time::timeout(SCRIPT_WAIT, answer)
            .await
            .map_err(|_| "The browser's cookie store did not answer in time.".to_string())?
            .map_err(|_| "The browser's cookie store closed before it answered.".to_string())?
    }

    /// Runs `work` on the main thread with the teammate's cookie store: its
    /// open page's, or the one its id names when its browser is closed.
    fn in_store(
        &self,
        persona_id: &str,
        work: impl FnOnce(&apple::Store) + Send + 'static,
    ) -> Result<(), String> {
        let ran = if let Ok(page) = self.window(persona_id) {
            page.with_webview(move |view| {
                // SAFETY: `with_webview` runs this on the main thread with
                // the page's live WKWebView.
                work(&unsafe { apple::page_store(view.inner()) });
            })
        } else {
            let id = hotline_core::browser::store_id(persona_id);
            self.app.run_on_main_thread(move || {
                if let Some(store) = apple::store_by_id(id) {
                    work(&store);
                }
            })
        };
        ran.map_err(|error| error.to_string())
    }
}

/// WebKit's cookie store, on the main thread.
mod apple {
    use super::{Cookie, Reply, on_site};
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{
        NSArray, NSDate, NSDictionary, NSHTTPCookie, NSHTTPCookieDomain, NSHTTPCookieExpires,
        NSHTTPCookieName, NSHTTPCookiePath, NSHTTPCookieSameSitePolicy, NSHTTPCookieSecure,
        NSHTTPCookieValue, NSString, NSUUID,
    };
    use objc2_web_kit::{WKHTTPCookieStore, WKWebView, WKWebsiteDataStore};
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::rc::Rc;

    pub(super) type Store = Retained<WKHTTPCookieStore>;

    /// The cookie store of a page's web view.
    ///
    /// SAFETY: on the main thread, with `view` a live `WKWebView`.
    pub(super) unsafe fn page_store(view: *mut c_void) -> Store {
        // SAFETY: as the function's own contract above.
        let view = unsafe { &*view.cast::<WKWebView>() };
        // SAFETY: plain WebKit getters on its own thread.
        unsafe { view.configuration().websiteDataStore().httpCookieStore() }
    }

    /// The teammate's persistent store by its id, as its page was made with.
    pub(super) fn store_by_id(id: [u8; 16]) -> Option<Store> {
        let main = MainThreadMarker::new()?;
        // SAFETY: a nonzero id (a hash), on the main thread.
        Some(unsafe {
            WKWebsiteDataStore::dataStoreForIdentifier(&NSUUID::from_bytes(id), main)
                .httpCookieStore()
        })
    }

    /// The answer, once, when the last of `count` callbacks has come back.
    fn countdown(count: usize, reply: Reply) -> impl Fn() + 'static {
        let left = Rc::new(Cell::new(count));
        let reply = Rc::new(RefCell::new(Some(reply)));
        move || {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0
                && let Some(reply) = reply.borrow_mut().take()
            {
                let _ = reply.send(Ok(()));
            }
        }
    }

    fn made(cookie: &Cookie) -> Option<Retained<NSHTTPCookie>> {
        let text = |value: &str| NSString::from_str(value);
        let yes = || Retained::into_super(Retained::into_super(text("TRUE")));
        // SAFETY: Foundation's own key constants, read-only.
        let mut keys: Vec<&NSString> = unsafe {
            vec![
                NSHTTPCookieName,
                NSHTTPCookieValue,
                NSHTTPCookieDomain,
                NSHTTPCookiePath,
            ]
        };
        let mut values: Vec<Retained<AnyObject>> = [
            text(&cookie.name),
            text(&cookie.value),
            text(&cookie.domain),
            text(&cookie.path),
        ]
        .into_iter()
        .map(Retained::into_super)
        .map(Retained::into_super)
        .collect();
        if let Some(expires) = cookie.expires {
            // SAFETY: as above.
            keys.push(unsafe { NSHTTPCookieExpires });
            values.push(Retained::into_super(Retained::into_super(
                NSDate::dateWithTimeIntervalSince1970(expires),
            )));
        }
        if cookie.secure {
            // SAFETY: as above.
            keys.push(unsafe { NSHTTPCookieSecure });
            values.push(yes());
        }
        // Foundation has no constant for it, and reads this key.
        let http_only = text("HttpOnly");
        if cookie.http_only {
            keys.push(&http_only);
            values.push(yes());
        }
        // `NSHTTPCookieSameSiteLax` and `…Strict` are these two words.
        let same_site = match cookie.same_site.as_deref() {
            Some("Lax") => Some("lax"),
            Some("Strict") => Some("strict"),
            _ => None,
        };
        if let Some(policy) = same_site {
            // SAFETY: as above.
            keys.push(unsafe { NSHTTPCookieSameSitePolicy });
            values.push(Retained::into_super(Retained::into_super(text(policy))));
        }
        let values: Vec<&AnyObject> = values.iter().map(|value| &**value).collect();
        let properties = NSDictionary::from_slices(&keys, &values);
        // SAFETY: string keys to strings and a date, as the method takes.
        unsafe { NSHTTPCookie::cookieWithProperties(&properties) }
    }

    pub(super) fn set(store: &Store, cookies: &[Cookie], reply: Reply) {
        let cookies: Vec<_> = cookies.iter().filter_map(made).collect();
        if cookies.is_empty() {
            let _ = reply.send(Err("WebKit took none of the lent cookies.".into()));
            return;
        }
        let done = RcBlock::new(countdown(cookies.len(), reply));
        for cookie in &cookies {
            // SAFETY: a cookie WebKit made, and a block it keeps until called.
            unsafe { store.setCookie_completionHandler(cookie, Some(&done)) };
        }
    }

    pub(super) fn delete(store: &Store, site: String, reply: Reply) {
        let kept = store.clone();
        let reply = RefCell::new(Some(reply));
        let listed = RcBlock::new(move |all: NonNull<NSArray<NSHTTPCookie>>| {
            let Some(reply) = reply.borrow_mut().take() else {
                return;
            };
            // SAFETY: WebKit hands a live array.
            let all = unsafe { all.as_ref() };
            let ours: Vec<_> = all
                .iter()
                .filter(|cookie| on_site(&cookie.domain().to_string(), &site))
                .collect();
            if ours.is_empty() {
                let _ = reply.send(Ok(()));
                return;
            }
            let done = RcBlock::new(countdown(ours.len(), reply));
            for cookie in &ours {
                // SAFETY: a cookie from this store, and a block it keeps.
                unsafe { kept.deleteCookie_completionHandler(cookie, Some(&done)) };
            }
        });
        // SAFETY: a block WebKit keeps until it is called.
        unsafe { store.getAllCookies(&listed) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_cookie_is_on_its_site_and_the_sites_under_it_only() {
        assert!(on_site(".github.com", "github.com"));
        assert!(on_site("github.com", "github.com"));
        assert!(on_site("Gist.GitHub.com", "github.com"));
        assert!(!on_site("evilgithub.com", "github.com"));
        assert!(!on_site("github.com.evil.dev", "github.com"));
    }

    #[test]
    fn a_lent_cookie_needs_a_name_a_value_and_a_domain() {
        let cookie = Cookie::of(&json!({
            "name": "sid", "value": "v", "domain": ".porkbun.com",
            "expires": -1, "httpOnly": true, "sameSite": "Lax"
        }))
        .unwrap();
        assert_eq!(cookie.path, "/");
        assert_eq!(cookie.expires, None, "a session cookie keeps no expiry");
        assert!(cookie.http_only && !cookie.secure);
        assert!(Cookie::of(&json!({"name": "sid", "value": "v"})).is_none());
        assert!(Cookie::of(&json!({"name": "sid", "domain": "a.test"})).is_none());
    }
}
