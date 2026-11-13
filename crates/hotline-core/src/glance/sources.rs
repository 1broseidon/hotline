//! The web pages a finished step opened, or a search listed, as the step's
//! own output names them: written on the step once, so the desk and the
//! phone count a turn's sources alike (the window's `sourcesOf`). A page is
//! never fetched again to fill a gap.

use super::{command_of, group_of, site_of, url_in};
use crate::contract::WebSource;

/// The most results of one search a step keeps as its sources.
const MOST_FOUND: usize = 10;

/// The longest page title a source keeps.
const SOURCE_TITLE_CHARS: usize = 160;

/// The teammate's browsers, by the names their tools are called: its
/// computer's (`computer__browser` on Hotline Agent, `mcp__Computer__browser`
/// through an ACP agent's MCP client, as Claude Code writes it) and the
/// desk's (`browser`, `mcp__hotline__browser`). A harness that writes its own
/// words for a call in place of the tool's answer (Antigravity's
/// `hotline_browser` says "Navigate to github.com") names no page it opened.
const BROWSERS: &[&str] = &[
    "computer__browser",
    "mcp__computer__browser",
    "browser",
    "mcp__hotline__browser",
];

/// The web pages a finished step opened or found, as its own output names
/// them; nothing for any other step. A fetch is the page in its title, with
/// the page's title when the tool writes it first (`web_fetch` does, then a
/// blank line); a search is the results its output lists, each `N. title`
/// over its address (Hotline's `web_search`) or `title (address)` (Claude's),
/// marked found, since listing a page is not opening it; a browser's step is
/// the page its answer says it went to. A "title" that is only the site's
/// name is no title.
pub fn sources_of(title: &str, kind: &str, output: &str) -> Vec<WebSource> {
    let first = title.split([':', ' ']).next().unwrap_or_default();
    let searched = group_of(kind, title) == "web"
        || [first, kind]
            .iter()
            .any(|name| name.trim_start_matches("mcp__hotline__") == "web_search");
    if searched {
        return listed(output);
    }
    if [first, kind]
        .iter()
        .any(|name| BROWSERS.contains(&name.to_ascii_lowercase().as_str()))
    {
        return browsed(output).into_iter().collect();
    }
    // A command that fetched a page (`curl` or `wget` with an address) read
    // it, as a fetch does; `gh api` and the like are an API, not a page.
    if group_of(kind, title) == "execute" {
        let command = command_of(title);
        let fetches = command
            .split(['|', ';', '&'])
            .map(str::trim)
            .filter(|part| {
                let program = part.split_whitespace().next().unwrap_or_default();
                matches!(
                    program.rsplit('/').next().unwrap_or_default(),
                    "curl" | "wget"
                )
            })
            .filter_map(|part| part.split_whitespace().find_map(url_in));
        let mut read: Vec<WebSource> = Vec::new();
        for url in fetches {
            if !read.iter().any(|known| known.url == url) {
                read.push(opened(url, None));
            }
        }
        return read;
    }
    if group_of(kind, title) != "fetch" {
        return Vec::new();
    }
    let Some(url) = url_in(title) else {
        return Vec::new();
    };
    let named = title.starts_with("web_fetch")
        && output
            .lines()
            .nth(1)
            .is_some_and(|line| line.trim().is_empty());
    let page = named
        .then(|| output.lines().next())
        .flatten()
        .and_then(|line| shown(&url, line));
    vec![opened(url, page)]
}

/// A page opened, not only listed.
fn opened(url: String, title: Option<String>) -> WebSource {
    WebSource {
        url,
        title,
        found: false,
    }
}

/// A page's title as a source keeps it: none when it is only the site's name
/// or an address.
fn shown(url: &str, name: &str) -> Option<String> {
    let name = name.trim();
    let site = site_of(url).unwrap_or_default();
    (!name.is_empty() && !name.eq_ignore_ascii_case(&site) && !name.contains("://"))
        .then(|| name.chars().take(SOURCE_TITLE_CHARS).collect::<String>())
}

/// The results a search's output lists, each once, the first
/// [`MOST_FOUND`] of them.
fn listed(output: &str) -> Vec<WebSource> {
    let mut found: Vec<WebSource> = Vec::new();
    let mut lines = output.lines().peekable();
    while let Some(line) = lines.next() {
        let line = line.trim();
        let hit = if let Some((number, name)) = line.split_once(". ")
            && number.chars().all(|c| c.is_ascii_digit())
            && !number.is_empty()
            && let Some(next) = lines.peek()
            && let Some(url) = url_in(next.trim()).filter(|url| url == next.trim())
        {
            lines.next();
            Some((url, name.to_string()))
        } else if let Some(open) = line.rfind(" (http")
            && line.ends_with(')')
            && let Some(url) = url_in(&line[open + 2..line.len() - 1])
        {
            Some((url, line[..open].to_string()))
        } else {
            None
        };
        if let Some((url, name)) = hit
            && !found.iter().any(|known| known.url == url)
        {
            found.push(WebSource {
                title: shown(&url, &name),
                url,
                found: true,
            });
        }
        if found.len() == MOST_FOUND {
            break;
        }
    }
    found
}

/// The page a browser's step went to, as its answer says: the computer's
/// browser answers a navigation "navigated to <address>" and a new tab
/// "opened new tab at <address>", either perhaps followed by "; " and a note
/// (hotline-computer's `src/browser.rs`); the desk's browser heads every
/// answer, an act's after its note, with "Page: <title>" over "URL:
/// <address>" (`session/browser.rs`'s `page_line`), so a click that changed
/// the page names the new one. A click on the computer's browser ("clicked
/// e12"), its back and forward, and a list of its tabs name no page opened.
fn browsed(output: &str) -> Option<WebSource> {
    let head = output.trim_start().lines().next().unwrap_or_default();
    for went in ["navigated to ", "opened new tab at "] {
        if let Some(rest) = head.strip_prefix(went) {
            let address = rest.split_whitespace().next()?.trim_end_matches(';');
            return url_in(address).map(|url| opened(url, None));
        }
    }
    let mut lines = output.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(name) = line.strip_prefix("Page: ")
            && let Some(url) = lines
                .peek()
                .and_then(|next| next.strip_prefix("URL: "))
                .and_then(|address| url_in(address.trim()))
        {
            let title = (name.trim() != "(no title)")
                .then(|| shown(&url, name))
                .flatten();
            return Some(opened(url, title));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::sources_of;
    use crate::contract::WebSource;

    fn page(url: &str, title: Option<&str>, found: bool) -> WebSource {
        WebSource {
            url: url.into(),
            title: title.map(str::to_string),
            found,
        }
    }

    #[test]
    fn a_fetched_page_is_its_address_and_the_title_its_tool_wrote_first() {
        assert_eq!(
            sources_of(
                "web_fetch: https://en.wikipedia.org/wiki/SSX_Tricky",
                "fetch",
                "SSX Tricky - Wikipedia\n\n[Jump to content](#bodyContent)\n",
            ),
            vec![page(
                "https://en.wikipedia.org/wiki/SSX_Tricky",
                Some("SSX Tricky - Wikipedia"),
                false
            )]
        );
        // Claude's fetch answers with its own summary, which is not the page's title.
        assert_eq!(
            sources_of(
                "Fetch https://www.onorca.dev/docs/mobile",
                "fetch",
                "# Orca Relay: What the Documentation Actually Says\n\nBased on…",
            ),
            vec![page("https://www.onorca.dev/docs/mobile", None, false)]
        );
        // Cursor's fetch, as it titles one on George's desk (tape 435e51a4, line 300).
        assert_eq!(
            sources_of(
                "Web Fetch: https://rentamac.io/mac-mini-history/",
                "fetch",
                ""
            ),
            vec![page("https://rentamac.io/mac-mini-history/", None, false)]
        );
        // A step that is not the web names no pages, whatever its output says.
        assert_eq!(
            sources_of("cat links.txt", "execute", "Docs (https://a.dev/x)"),
            Vec::new()
        );
    }

    #[test]
    fn a_web_search_names_the_results_its_output_lists() {
        let hotline = "1. How should I care for my goldfish? - RSPCA\n   https://kb.rspca.org.au/goldfish\n   Goldfish are social…\n\n2. Keeping Fish as Pets\n   https://www.rspca.org.uk/fish\n   Goldfish were…\n";
        assert_eq!(
            sources_of("mcp__hotline__web_search: goldfish care", "search", hotline),
            vec![
                page(
                    "https://kb.rspca.org.au/goldfish",
                    Some("How should I care for my goldfish? - RSPCA"),
                    true
                ),
                page(
                    "https://www.rspca.org.uk/fish",
                    Some("Keeping Fish as Pets"),
                    true
                ),
            ]
        );
        let claude = "apps.apple.com (https://apps.apple.com/us/app/orca-ide/id6766130217)\nMobile companion (https://www.onorca.dev/docs/mobile)\nBased on the search results, here's what I found (with notes)\n";
        assert_eq!(
            sources_of("Search \"Orca app relay\"", "fetch", claude),
            vec![
                // A site's name is not a page title.
                page(
                    "https://apps.apple.com/us/app/orca-ide/id6766130217",
                    None,
                    true
                ),
                page(
                    "https://www.onorca.dev/docs/mobile",
                    Some("Mobile companion"),
                    true
                ),
            ]
        );
    }

    /// Goldie's steps in its computer's browser on George's desk (tape
    /// 79d12f56, lines 3048-3054), as Hotline Agent titles them.
    #[test]
    fn a_page_its_computers_browser_went_to_is_a_page_opened() {
        let goldie = |output: &str| sources_of("computer__browser", "computer__browser", output);
        assert_eq!(
            goldie(
                "navigated to https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition"
            ),
            vec![page(
                "https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition",
                None,
                false
            )]
        );
        // Reading the page says its title but not its address: nothing told.
        assert_eq!(
            goldie("page: NVIDIA Announces CEO Transition | NVIDIA Newsroom\nNewsroom\n"),
            Vec::new()
        );
        // A page that answered with an error status is still open in its tab.
        assert_eq!(
            goldie(
                "opened new tab at https://porkbun.com/account/login; the server answered with an error status; browser text shows the page"
            ),
            vec![page("https://porkbun.com/account/login", None, false)]
        );
        for nothing in [
            "clicked e12",
            "back",
            r#"[{"current":true,"index":0,"title":"porkbun.com","url":"https://porkbun.com/"}]"#,
            "navigated to file:///home/agent/gull-point/index.html",
        ] {
            assert_eq!(goldie(nothing), Vec::new(), "{nothing}");
        }
        // Through Claude Code's MCP client the same tool is named by its server.
        assert_eq!(
            sources_of(
                "mcp__Computer__browser",
                "other",
                "navigated to https://github.com/notifications"
            ),
            vec![page("https://github.com/notifications", None, false)]
        );
    }

    /// The desk's browser on George's desk: through Claude Code (tape
    /// 1e615299, line 176) and on Hotline Agent (tape 9f959998, line 225).
    #[test]
    fn the_desks_browser_names_the_page_it_is_on_after_any_act() {
        assert_eq!(
            sources_of(
                "mcp__hotline__browser",
                "other",
                "Page: Apple I - Wikipedia\nURL: https://en.wikipedia.org/wiki/Apple_I"
            ),
            vec![page(
                "https://en.wikipedia.org/wiki/Apple_I",
                Some("Apple I - Wikipedia"),
                false
            )]
        );
        assert_eq!(
            sources_of(
                "browser",
                "browser",
                "On button \"Continue\".\nPage: Sign in · Proximux\nURL: https://auth.proximux.dev/login\nRead the page again with text to see what changed; refs you have still mean the same elements, and one whose element is gone will say so."
            ),
            vec![page(
                "https://auth.proximux.dev/login",
                Some("Sign in · Proximux"),
                false
            )]
        );
        assert_eq!(
            sources_of(
                "browser",
                "browser",
                "Page: (no title)\nURL: https://example.org/blank"
            ),
            vec![page("https://example.org/blank", None, false)]
        );
        // A picture names no page; Antigravity's own words for a call are not its answer.
        assert_eq!(
            sources_of(
                "browser",
                "browser",
                "This picture is yours to look at; the person does not see it.\n[image image/png, 892 KB]"
            ),
            Vec::new()
        );
        assert_eq!(
            sources_of(
                "hotline_browser",
                "other",
                "Navigate to http://127.0.0.1:4520/"
            ),
            Vec::new()
        );
    }
}
