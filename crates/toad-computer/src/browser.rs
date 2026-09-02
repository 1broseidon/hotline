use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::cdp::browser_protocol::network::CookieParam;
use chromiumoxide::cdp::browser_protocol::page::HandleJavaScriptDialogParams;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::Config;

pub const NO_BROWSER: &str = "This computer has no browser installed.";

#[derive(Clone)]
pub struct BrowserManager {
    config: Arc<Config>,
    session: Arc<Mutex<Option<BrowserSession>>>,
}

struct BrowserSession {
    browser: Browser,
    current: usize,
    _handler: tokio::task::JoinHandle<()>,
}

impl BrowserManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            session: Arc::new(Mutex::new(None)),
        }
    }

    async fn with_session<T, F>(&self, operation: F) -> Result<T, String>
    where
        F: for<'a> FnOnce(
            &'a mut BrowserSession,
        ) -> Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>,
    {
        let mut session = self.session.lock().await;
        if session.is_none() {
            *session = Some(self.launch().await?);
        }
        operation(session.as_mut().expect("created above")).await
    }

    async fn launch(&self) -> Result<BrowserSession, String> {
        let executable = find_on_path("chromium").ok_or_else(|| NO_BROWSER.to_owned())?;
        let profile = self.config.home.join(".toad/browser");
        tokio::fs::create_dir_all(&profile)
            .await
            .map_err(|error| format!("create browser profile: {error}"))?;
        let browser_config = BrowserConfig::builder()
            .chrome_executable(executable)
            .with_head()
            .no_sandbox()
            .port(9222)
            .viewport(None)
            .user_data_dir(profile)
            .env("DISPLAY", self.config.display.clone())
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .build()
            .map_err(|error| format!("browser: {error}"))?;
        let (browser, mut handler) = Browser::launch(browser_config)
            .await
            .map_err(|error| format!("browser: {error}"))?;
        let handler = tokio::spawn(async move { while handler.next().await.is_some() {} });
        if browser.pages().await.map_err(browser_error)?.is_empty() {
            browser
                .new_page("about:blank")
                .await
                .map_err(browser_error)?;
        }
        Ok(BrowserSession {
            browser,
            current: 0,
            _handler: handler,
        })
    }

    pub async fn navigate(&self, url: &str) -> Result<String, String> {
        if url.is_empty() {
            return Err("url is required".to_owned());
        }
        let url = url.to_owned();
        self.with_session(|session| {
            Box::pin(async move {
                current_page(session)
                    .await?
                    .goto(url.as_str())
                    .await
                    .map_err(browser_error)?;
                Ok(format!("navigated to {url}"))
            })
        })
        .await
    }

    pub async fn text(&self) -> Result<String, String> {
        self.with_session(|session| Box::pin(async move {
            let page = current_page(session).await?;
            let script = r#"(() => {
                document.querySelectorAll('[data-toad-ref]').forEach(e => e.removeAttribute('data-toad-ref'));
                const interesting = 'a,button,input,select,textarea,[role],h1,h2,h3,h4,h5,h6';
                const lines = [];
                let next = 1;
                for (const element of document.querySelectorAll(interesting)) {
                    const ref = `e${next++}`;
                    element.setAttribute('data-toad-ref', ref);
                    const tag = element.tagName.toLowerCase();
                    const role = element.getAttribute('role') || ({a:'link',button:'button',input:element.type || 'input',select:'combobox',textarea:'textbox'}[tag] || tag);
                    const name = element.getAttribute('aria-label') || element.innerText || element.value || element.placeholder || '';
                    if (name.trim()) lines.push(`[${ref}] [${role}] ${name.trim().replace(/\s+/g, ' ')}`);
                }
                const body = document.body ? document.body.innerText.trim() : '';
                return [`page: ${document.title}`, body, ...lines].filter(Boolean).join('\n');
            })()"#;
            evaluate_string(&page, script).await
        }))
        .await
    }

    pub async fn links(&self) -> Result<String, String> {
        self.with_session(|session| Box::pin(async move {
            let page = current_page(session).await?;
            let value = evaluate_value(
                &page,
                "[...document.querySelectorAll('a[href]')].map(a => ({text:(a.innerText||'').trim(),href:a.href}))",
            )
            .await?;
            serde_json::to_string(&value).map_err(|error| error.to_string())
        }))
        .await
    }

    pub async fn eval(&self, javascript: &str) -> Result<String, String> {
        if javascript.is_empty() {
            return Err("js is required".to_owned());
        }
        let javascript = javascript.to_owned();
        self.with_session(|session| {
            Box::pin(async move {
                let value = evaluate_value(&current_page(session).await?, &javascript).await?;
                serde_json::to_string(&value).map_err(|error| error.to_string())
            })
        })
        .await
    }

    pub async fn click_ref(&self, reference: &str, double: bool) -> Result<String, String> {
        let selector = ref_selector(reference)?;
        let reference = reference.to_owned();
        self.with_session(|session| Box::pin(async move {
            let page = current_page(session).await?;
            let element = page.find_element(selector).await.map_err(browser_error)?;
            if double {
                element
                    .call_js_fn("function(){ this.dispatchEvent(new MouseEvent('dblclick', {bubbles:true})); }", true)
                    .await
                    .map_err(browser_error)?;
            } else {
                element.click().await.map_err(browser_error)?;
            }
            Ok(format!("clicked {reference}"))
        }))
        .await
    }

    pub async fn fill(&self, reference: &str, text: &str) -> Result<String, String> {
        self.element_script(
            reference,
            &format!(
                "function(){{this.focus();this.value={};this.dispatchEvent(new Event('input',{{bubbles:true}}));this.dispatchEvent(new Event('change',{{bubbles:true}}));}}",
                json!(text)
            ),
            format!("filled {reference}"),
        )
        .await
    }

    pub async fn select(&self, reference: &str, value: &str) -> Result<String, String> {
        self.element_script(
            reference,
            &format!(
                "function(){{this.value={};this.dispatchEvent(new Event('change',{{bubbles:true}}));}}",
                json!(value)
            ),
            format!("selected {value:?} on {reference}"),
        )
        .await
    }

    pub async fn check(&self, reference: &str, checked: bool) -> Result<String, String> {
        self.element_script(
            reference,
            &format!(
                "function(){{this.checked={checked};this.dispatchEvent(new Event('input',{{bubbles:true}}));this.dispatchEvent(new Event('change',{{bubbles:true}}));}}"
            ),
            format!("{} {reference}", if checked { "checked" } else { "unchecked" }),
        )
        .await
    }

    pub async fn hover(&self, reference: &str) -> Result<String, String> {
        let selector = ref_selector(reference)?;
        let reference = reference.to_owned();
        self.with_session(|session| {
            Box::pin(async move {
                current_page(session)
                    .await?
                    .find_element(selector)
                    .await
                    .map_err(browser_error)?
                    .hover()
                    .await
                    .map_err(browser_error)?;
                Ok(format!("hovered {reference}"))
            })
        })
        .await
    }

    async fn element_script(
        &self,
        reference: &str,
        script: &str,
        result: String,
    ) -> Result<String, String> {
        let selector = ref_selector(reference)?;
        let script = script.to_owned();
        self.with_session(|session| {
            Box::pin(async move {
                current_page(session)
                    .await?
                    .find_element(selector)
                    .await
                    .map_err(browser_error)?
                    .call_js_fn(&script, true)
                    .await
                    .map_err(browser_error)?;
                Ok(result)
            })
        })
        .await
    }

    pub async fn tabs(&self) -> Result<String, String> {
        self.with_session(|session| {
            Box::pin(async move {
                let pages = session.browser.pages().await.map_err(browser_error)?;
                let mut tabs = Vec::new();
                for (index, page) in pages.iter().enumerate() {
                    tabs.push(json!({
                        "index": index,
                        "title": page.get_title().await.map_err(browser_error)?.unwrap_or_default(),
                        "url": page.url().await.map_err(browser_error)?.unwrap_or_default(),
                        "current": index == session.current,
                    }));
                }
                serde_json::to_string(&tabs).map_err(|error| error.to_string())
            })
        })
        .await
    }

    pub async fn tab_new(&self, url: &str) -> Result<String, String> {
        let url = url.to_owned();
        self.with_session(|session| {
            Box::pin(async move {
                let page = session
                    .browser
                    .new_page(if url.is_empty() {
                        "about:blank"
                    } else {
                        url.as_str()
                    })
                    .await
                    .map_err(browser_error)?;
                let pages = session.browser.pages().await.map_err(browser_error)?;
                session.current = pages
                    .iter()
                    .position(|candidate| candidate.target_id() == page.target_id())
                    .unwrap_or_else(|| pages.len().saturating_sub(1));
                Ok(if url.is_empty() {
                    "opened new tab".to_owned()
                } else {
                    format!("opened new tab at {url}")
                })
            })
        })
        .await
    }

    pub async fn tab_select(&self, index: usize) -> Result<String, String> {
        self.with_session(|session| {
            Box::pin(async move {
                let pages = session.browser.pages().await.map_err(browser_error)?;
                let page = pages
                    .get(index)
                    .ok_or_else(|| format!("tab {index} not found"))?;
                page.bring_to_front().await.map_err(browser_error)?;
                session.current = index;
                Ok(format!("selected tab {index}"))
            })
        })
        .await
    }

    pub async fn tab_close(&self, index: Option<usize>) -> Result<String, String> {
        self.with_session(|session| {
            Box::pin(async move {
                let index = index.unwrap_or(session.current);
                let mut pages = session.browser.pages().await.map_err(browser_error)?;
                if index >= pages.len() {
                    return Err(format!("tab {index} not found"));
                }
                pages.remove(index).close().await.map_err(browser_error)?;
                let remaining = session.browser.pages().await.map_err(browser_error)?;
                if remaining.is_empty() {
                    session
                        .browser
                        .new_page("about:blank")
                        .await
                        .map_err(browser_error)?;
                    session.current = 0;
                } else {
                    session.current = session.current.min(remaining.len() - 1);
                }
                Ok(format!("closed tab {index}"))
            })
        })
        .await
    }

    pub async fn upload(&self, reference: &str, path: &Path) -> Result<String, String> {
        let selector = ref_selector(reference)?;
        let path = path.to_string_lossy().into_owned();
        self.with_session(|session| {
            Box::pin(async move {
                let page = current_page(session).await?;
                let element = page.find_element(selector).await.map_err(browser_error)?;
                page.execute(
                    SetFileInputFilesParams::builder()
                        .backend_node_id(element.backend_node_id)
                        .files(vec![path.clone()])
                        .build()
                        .map_err(|error| error.to_string())?,
                )
                .await
                .map_err(browser_error)?;
                Ok(format!("uploaded {path}"))
            })
        })
        .await
    }

    pub async fn dialog(&self, accept: bool, text: &str) -> Result<String, String> {
        let text = text.to_owned();
        self.with_session(|session| {
            Box::pin(async move {
                current_page(session)
                    .await?
                    .execute({
                        let builder = HandleJavaScriptDialogParams::builder().accept(accept);
                        let builder = if accept && !text.is_empty() {
                            builder.prompt_text(&text)
                        } else {
                            builder
                        };
                        builder.build().map_err(|error| error.to_string())?
                    })
                    .await
                    .map_err(browser_error)?;
                Ok(if accept {
                    "accepted dialog"
                } else {
                    "dismissed dialog"
                }
                .to_owned())
            })
        })
        .await
    }

    pub async fn history(&self, action: &str) -> Result<String, String> {
        let action = action.to_owned();
        self.with_session(|session| {
            Box::pin(async move {
                let page = current_page(session).await?;
                match action.as_str() {
                    "back" => {
                        page.evaluate("history.back()")
                            .await
                            .map_err(browser_error)?;
                    }
                    "forward" => {
                        page.evaluate("history.forward()")
                            .await
                            .map_err(browser_error)?;
                    }
                    "reload" => {
                        page.reload().await.map_err(browser_error)?;
                    }
                    _ => return Err(format!("unknown browser history action {action:?}")),
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(action.to_owned())
            })
        })
        .await
    }

    pub async fn page_text_if_running(&self) -> Option<String> {
        let mut session = self.session.lock().await;
        let session = session.as_mut()?;
        let page = current_page(session).await.ok()?;
        evaluate_string(&page, "document.body ? document.body.innerText : ''")
            .await
            .ok()
    }

    pub async fn cookies(&self) -> Result<Value, String> {
        self.with_session(|session| {
            Box::pin(async move {
                let cookies = session.browser.get_cookies().await.map_err(browser_error)?;
                serde_json::to_value(cookies).map_err(|error| error.to_string())
            })
        })
        .await
    }

    pub async fn local_storage(&self) -> Result<Value, String> {
        self.with_session(|session| {
            Box::pin(async move {
                evaluate_value(
                    &current_page(session).await?,
                    "Object.fromEntries(Object.entries(localStorage))",
                )
                .await
            })
        })
        .await
    }

    pub async fn restore(&self, cookies: Value, local_storage: Value) -> Result<(), String> {
        let cookies: Vec<CookieParam> =
            serde_json::from_value(cookies).map_err(|error| format!("saved cookies: {error}"))?;
        self.with_session(|session| Box::pin(async move {
            session.browser.set_cookies(cookies).await.map_err(browser_error)?;
            let storage = serde_json::to_string(&local_storage).map_err(|error| error.to_string())?;
            current_page(session)
                .await?
                .evaluate(format!(
                    "(() => {{ localStorage.clear(); for (const [key,value] of Object.entries({storage})) localStorage.setItem(key,value); }})()"
                ))
                .await
                .map_err(browser_error)?;
            Ok(())
        }))
        .await
    }
}

async fn current_page(session: &mut BrowserSession) -> Result<Page, String> {
    let pages = session.browser.pages().await.map_err(browser_error)?;
    if pages.is_empty() {
        return session
            .browser
            .new_page("about:blank")
            .await
            .map_err(browser_error);
    }
    session.current = session.current.min(pages.len() - 1);
    Ok(pages[session.current].clone())
}

async fn evaluate_value(page: &Page, script: &str) -> Result<Value, String> {
    page.evaluate(script)
        .await
        .map_err(browser_error)?
        .into_value()
        .map_err(browser_error)
}

async fn evaluate_string(page: &Page, script: &str) -> Result<String, String> {
    let value = evaluate_value(page, script).await?;
    Ok(value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned))
}

fn ref_selector(reference: &str) -> Result<String, String> {
    if reference.is_empty() {
        return Err("ref is required".to_owned());
    }
    if !reference
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("ref is invalid".to_owned());
    }
    Ok(format!("[data-toad-ref=\"{reference}\"]"))
}

fn browser_error(error: impl std::fmt::Display) -> String {
    format!("browser: {error}")
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .find_map(|directory| {
            let candidate = Path::new(directory).join(name);
            candidate.is_file().then_some(candidate)
        })
}
