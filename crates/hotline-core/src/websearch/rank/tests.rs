//! Each rule of the ranking, on a fixture shaped like what went wrong live.

use super::*;
use crate::contract::WebSearchProvider as P;

fn hit(title: &str, url: &str, snippet: &str) -> Hit {
    Hit {
        title: title.into(),
        url: url.into(),
        snippet: snippet.into(),
        ..Default::default()
    }
}

fn src(provider: P, hits: Vec<Hit>) -> Source {
    Source {
        provider,
        keyed: false,
        hits,
    }
}

fn run(query: &str, sources: Vec<Source>) -> Ranked {
    rank(&Query::parse(query), sources, 10)
}

fn urls(ranked: &Ranked) -> Vec<&str> {
    ranked.hits.iter().map(|hit| hit.url.as_str()).collect()
}

const SENTENCE: &str = "A long enough sentence about the subject that reads as real page text.";

fn plain(title: &str, url: &str) -> Hit {
    hit(title, url, SENTENCE)
}

#[test]
fn two_providers_agreeing_outrank_one_providers_first_place() {
    let ranked = run(
        "subject",
        vec![
            src(
                P::Parallel,
                vec![
                    plain("Alone", "https://a.example/1"),
                    plain("Both", "https://b.example/2"),
                ],
            ),
            src(
                P::Exa,
                vec![
                    plain("Other", "https://c.example/3"),
                    plain("Both", "https://b.example/2"),
                ],
            ),
        ],
    );
    assert_eq!(urls(&ranked)[0], "https://b.example/2");
}

#[test]
fn a_keyed_providers_list_counts_a_little_more() {
    let lists = |keyed: bool| {
        vec![
            src(
                P::Parallel,
                vec![plain("Parallel first", "https://a.example/1")],
            ),
            Source {
                keyed,
                ..src(P::Exa, vec![plain("Exa first", "https://z.example/1")])
            },
        ]
    };
    assert_eq!(
        urls(&run("subject", lists(false)))[0],
        "https://a.example/1"
    );
    assert_eq!(urls(&run("subject", lists(true)))[0], "https://z.example/1");
}

#[test]
fn a_tracking_parameter_does_not_make_a_second_result() {
    let ranked = run(
        "subject",
        vec![
            src(
                P::Parallel,
                vec![plain("Page", "https://example.com/page?utm_source=x")],
            ),
            src(P::Exa, vec![plain("Page", "http://www.example.com/page/")]),
        ],
    );
    assert_eq!(ranked.hits.len(), 1);
}

#[test]
fn language_and_mobile_mirrors_of_one_page_are_one_result() {
    let ranked = run(
        "why is the sky blue",
        vec![src(
            P::Parallel,
            vec![
                plain(
                    "Why is the sky blue?",
                    "https://spaceplace.nasa.gov/blue-sky/en/",
                ),
                plain(
                    "Why is the sky blue?",
                    "https://spaceplace.nasa.gov/blue-sky/sp/",
                ),
                plain(
                    "Why is the sky blue?",
                    "https://spaceplace.nasa.gov/blue-sky/es/",
                ),
                plain(
                    "Rayleigh scattering",
                    "https://en.wikipedia.org/wiki/Rayleigh_scattering",
                ),
                plain(
                    "Rayleigh scattering",
                    "https://de.wikipedia.org/wiki/Rayleigh_scattering",
                ),
                plain(
                    "Rayleigh scattering",
                    "https://en.m.wikipedia.org/wiki/Rayleigh_scattering",
                ),
                plain("Docs", "https://www.example.com/docs"),
                plain("Docs", "https://m.example.com/docs"),
            ],
        )],
    );
    assert_eq!(ranked.hits.len(), 3, "{:?}", urls(&ranked));
}

#[test]
fn the_same_title_on_one_domain_is_one_result() {
    let ranked = run(
        "module",
        vec![src(
            P::Parallel,
            vec![
                hit(
                    "github.com/acme/widget - Go Packages",
                    "https://pkg.go.dev/github.com/acme/widget",
                    "The widget module.",
                ),
                hit(
                    "github.com/acme/widget - Go Packages",
                    "https://pkg.go.dev/github.com/acme/widget@v1.2.0",
                    "Version one.",
                ),
                hit("A different page", "https://pkg.go.dev/other", SENTENCE),
            ],
        )],
    );
    assert_eq!(ranked.hits.len(), 2);
}

#[test]
fn the_same_title_across_domains_merges_only_when_the_snippets_are_the_same() {
    let text = "The blue sky is caused by Rayleigh scattering of sunlight by the gases in the atmosphere of Earth";
    let same = run(
        "sky",
        vec![src(
            P::Parallel,
            vec![
                hit("Why the sky is blue", "https://one.example/a", text),
                hit("Why the sky is blue", "https://two.example/b", text),
            ],
        )],
    );
    assert_eq!(same.hits.len(), 1);
    let different = run(
        "sky",
        vec![src(
            P::Parallel,
            vec![
                hit("Why the sky is blue", "https://one.example/a", text),
                hit(
                    "Why the sky is blue",
                    "https://two.example/b",
                    "A children's story about a bird that wanted to fly above the clouds one day",
                ),
            ],
        )],
    );
    assert_eq!(different.hits.len(), 2);
}

#[test]
fn a_merged_result_keeps_the_better_snippet() {
    let ranked = run(
        "everest",
        vec![
            src(
                P::Parallel,
                vec![hit(
                    "Everest",
                    "https://example.com/everest",
                    "Home | Menu | Subscribe",
                )],
            ),
            src(
                P::Exa,
                vec![hit(
                    "Everest",
                    "https://example.com/everest/",
                    "Everest is the highest mountain above sea level, in the Himalayas.",
                )],
            ),
        ],
    );
    assert_eq!(ranked.hits.len(), 1);
    assert!(ranked.hits[0].snippet.starts_with("Everest is the highest"));
}

#[test]
fn a_copy_in_the_queries_language_stands_for_its_mirror() {
    let query = "Quelle est l'altitude du mont Everest en mètres";
    assert_eq!(Query::parse(query).language.map(|l| l.code), Some("fr"));
    let ranked = run(
        query,
        vec![src(
            P::Parallel,
            vec![
                plain("Everest altitude", "https://example.com/en/everest"),
                hit(
                    "Everest altitude",
                    "https://fr.example.com/everest",
                    "L'Everest est la plus haute montagne du monde, avec une altitude de 8849 mètres au-dessus du niveau de la mer.",
                ),
            ],
        )],
    );
    assert_eq!(ranked.hits.len(), 1);
    assert_eq!(ranked.hits[0].url, "https://fr.example.com/everest");
}

#[test]
fn error_pages_are_dropped_and_the_working_copy_stays() {
    let ranked = run(
        "rfc 9110 http semantics",
        vec![src(
            P::Parallel,
            vec![
                hit(
                    "RFC 9110",
                    "https://www.ietf.org/rfc/rfc9110.html",
                    "301 Moved Permanently cloudflare",
                ),
                hit(
                    "RFC 9110: HTTP Semantics",
                    "https://www.rfc-editor.org/rfc/rfc9110",
                    "The Hypertext Transfer Protocol (HTTP) is a stateless application-level protocol.",
                ),
                hit("403 Forbidden", "https://x.example/rfc", ""),
            ],
        )],
    );
    assert_eq!(
        urls(&ranked),
        vec!["https://www.rfc-editor.org/rfc/rfc9110"]
    );
}

#[test]
fn a_result_with_nothing_readable_ranks_below_one_that_can_say_what_it_is() {
    let ranked = run(
        "subject",
        vec![src(
            P::Parallel,
            vec![
                hit(
                    "Tag page",
                    "https://news.example/a",
                    "Home · About · Contact · Subscribe · Log in",
                ),
                plain("Real page", "https://news.example/b"),
            ],
        )],
    );
    assert_eq!(urls(&ranked)[0], "https://news.example/b");
    assert_eq!(ranked.hits.len(), 2, "kept, just lower");
}

#[test]
fn a_domain_written_in_the_query_comes_first() {
    let ranked = run(
        "hotline.dev desktop app",
        vec![src(
            P::Parallel,
            vec![
                plain("Freshdesk Hotline", "https://freshdesk.com/hotline"),
                plain("hotline module", "https://pkg.go.dev/github.com/x/hotline"),
                plain("Hotline", "https://hotline.dev/"),
            ],
        )],
    );
    assert_eq!(urls(&ranked)[0], "https://hotline.dev/");
}

#[test]
fn a_one_word_query_that_is_a_sites_name_puts_that_site_first() {
    let ranked = run(
        "apple",
        vec![src(
            P::Parallel,
            vec![
                plain("apple - Wiktionary", "https://zh.wiktionary.org/wiki/apple"),
                plain("Apple", "https://www.apple.com/"),
                plain(
                    "Apple Inc. - Wikipedia",
                    "https://en.wikipedia.org/wiki/Apple_Inc.",
                ),
            ],
        )],
    );
    assert_eq!(urls(&ranked)[0], "https://www.apple.com/");
    assert_eq!(urls(&ranked)[1], "https://en.wikipedia.org/wiki/Apple_Inc.");
}

#[test]
fn social_and_video_hosts_lose_ground_unless_the_query_names_them() {
    let lists = || {
        vec![src(
            P::Parallel,
            vec![
                plain("Everest video", "https://www.facebook.com/watch/?v=1"),
                plain("Everest reel", "https://www.tiktok.com/@x/video/2"),
                plain(
                    "Mount Everest",
                    "https://www.britannica.com/place/Mount-Everest",
                ),
            ],
        )]
    };
    assert_eq!(
        urls(&run("everest", lists()))[0],
        "https://www.britannica.com/place/Mount-Everest"
    );
    assert_eq!(
        urls(&run("everest facebook", lists()))[0],
        "https://www.facebook.com/watch/?v=1"
    );
}

#[test]
fn wikipedia_in_the_queries_language_gets_a_lift() {
    let english = run(
        "Mount Everest",
        vec![src(
            P::Parallel,
            vec![
                plain("Everest guide", "https://www.facebook-free.example/everest"),
                plain(
                    "Mount Everest - Wikipedia",
                    "https://en.wikipedia.org/wiki/Mount_Everest",
                ),
            ],
        )],
    );
    assert_eq!(
        urls(&english)[0],
        "https://en.wikipedia.org/wiki/Mount_Everest"
    );
    let query = "Quelle est l'altitude du mont Everest en mètres";
    let french = run(
        query,
        vec![src(
            P::Parallel,
            vec![
                hit(
                    "Mount Everest - Wikipedia",
                    "https://en.wikipedia.org/wiki/Mount_Everest",
                    "Mount Everest is Earth's highest mountain above sea level, located in the Himalayas.",
                ),
                hit(
                    "Everest - Wikipédia",
                    "https://fr.wikipedia.org/wiki/Everest",
                    "L'Everest est le plus haut sommet du monde, situé dans l'Himalaya, à la frontière entre le Népal et la Chine.",
                ),
            ],
        )],
    );
    assert_eq!(urls(&french)[0], "https://fr.wikipedia.org/wiki/Everest");
}

#[test]
fn a_non_english_query_prefers_pages_in_its_language_and_english_is_not_penalised_for_english() {
    let query = "Quelle est l'altitude du mont Everest en mètres";
    let ranked = run(
        query,
        vec![src(
            P::Parallel,
            vec![
                hit(
                    "Everest height",
                    "https://a.example/everest",
                    "Mount Everest is Earth's highest mountain above sea level, located in the Himalayas of Nepal.",
                ),
                hit(
                    "Altitude de l'Everest",
                    "https://b.example/everest",
                    "L'Everest culmine à 8849 mètres d'altitude, ce qui en fait le plus haut sommet de la planète.",
                ),
            ],
        )],
    );
    assert_eq!(urls(&ranked)[0], "https://b.example/everest");
    let english = run(
        "how high is mount everest",
        vec![src(
            P::Parallel,
            vec![hit(
                "Everest height",
                "https://a.example/everest",
                "Mount Everest is Earth's highest mountain above sea level, located in the Himalayas of Nepal.",
            )],
        )],
    );
    assert_eq!(english.hits.len(), 1);
}

#[test]
fn a_comparison_demotes_a_package_named_for_both_and_lifts_a_page_about_both() {
    let ranked = run(
        "tokio vs async-std",
        vec![src(
            P::Parallel,
            vec![
                plain(
                    "tokio-async-std - crates.io",
                    "https://crates.io/crates/tokio-async-std",
                ),
                plain(
                    "tokio_async_std",
                    "https://docs.rs/tokio-async-std/latest/tokio_async_std/",
                ),
                hit(
                    "Tokio vs async-std: which runtime?",
                    "https://blog.example/runtimes",
                    "We compare tokio and async-std on throughput and ergonomics for async Rust code.",
                ),
            ],
        )],
    );
    assert_eq!(urls(&ranked)[0], "https://blog.example/runtimes");
    assert!(urls(&ranked)[1..].contains(&"https://crates.io/crates/tokio-async-std"));
    // A package page for one side is not demoted.
    let single = run(
        "tokio vs async-std",
        vec![src(
            P::Parallel,
            vec![
                plain("tokio", "https://docs.rs/tokio/latest/tokio/"),
                plain("Other", "https://x.example/y"),
            ],
        )],
    );
    assert_eq!(urls(&single)[0], "https://docs.rs/tokio/latest/tokio/");
}

#[test]
fn a_news_query_demotes_tag_and_index_pages_and_a_plain_one_does_not() {
    let lists = || {
        vec![src(
            P::Parallel,
            vec![
                plain("World news tag", "https://news.example/tag/world"),
                plain(
                    "Parliament votes",
                    "https://news.example/2026/10/06/parliament-votes",
                ),
            ],
        )]
    };
    assert_eq!(
        urls(&run("latest world news October 6 2026", lists()))[0],
        "https://news.example/2026/10/06/parliament-votes"
    );
    assert_eq!(
        urls(&run("world", lists()))[0],
        "https://news.example/tag/world"
    );
}

#[test]
fn a_distinctive_word_no_result_carries_is_said() {
    let ranked = run(
        "xqzflarnib 98421 protocol",
        vec![src(
            P::Parallel,
            vec![plain(
                "Clinical trial protocol",
                "https://trials.example/protocol.pdf",
            )],
        )],
    );
    assert_eq!(
        ranked.note.as_deref(),
        Some("No result mentions \"xqzflarnib\" or \"98421\"; these match only the other words.")
    );
    let found = run(
        "rust protocol",
        vec![src(
            P::Parallel,
            vec![plain("Rust protocol", "https://x.example/rust-protocol")],
        )],
    );
    assert_eq!(found.note, None);
}

// Round 3: news, snippets, and the miss warning ----------------------------------

fn on(text: &str) -> Query {
    Query::parse_on(text, chrono::NaiveDate::from_ymd_opt(2026, 10, 7).unwrap())
}

fn dated(title: &str, url: &str, snippet: &str, published: Option<&str>) -> Hit {
    Hit {
        published: published.map(str::to_string),
        ..hit(title, url, snippet)
    }
}

fn news(query: &str, list: Vec<Hit>) -> Vec<String> {
    let ranked = rank(&on(query), vec![src(P::Parallel, list)], 10);
    ranked.hits.into_iter().map(|h| h.url).collect()
}

const STORY: &str =
    "Officials said on Tuesday that talks continued through the night in the capital.";

#[test]
fn a_story_dated_in_the_window_beats_one_a_year_older() {
    // The older page is first from the provider; the date moves it down.
    let order = news(
        "latest world news October 7 2026",
        vec![
            dated(
                "Old",
                "https://a.example/news/old-story-from-2023",
                STORY,
                Some("2023-10-07"),
            ),
            dated(
                "New",
                "https://b.example/news/new-story-from-today",
                STORY,
                Some("2026-10-07T05:00:00Z"),
            ),
        ],
    );
    assert_eq!(order[0], "https://b.example/news/new-story-from-today");
}

#[test]
fn a_date_in_the_snippet_or_a_relative_time_counts_when_no_provider_gave_one() {
    let order = news(
        "latest world news",
        vec![
            plain("Old", "https://a.example/news/an-older-page-here"),
            hit(
                "Recent",
                "https://b.example/news/a-recent-page-here",
                "34 min ago Officials said talks continued through the night.",
            ),
        ],
    );
    assert_eq!(order[0], "https://b.example/news/a-recent-page-here");
    let order = news(
        "latest world news",
        vec![
            hit(
                "Anniversary",
                "https://a.example/news/the-anniversary-page",
                "On October 7, 2023 militants crossed the border in a surprise assault.",
            ),
            plain("Fresh", "https://b.example/news/a-fresh-page-here"),
        ],
    );
    assert_eq!(order[0], "https://b.example/news/a-fresh-page-here");
}

#[test]
fn an_events_name_is_not_a_date_and_does_not_make_a_window() {
    assert!(on("what caused the October 7 attack").window.is_none());
    assert!(on("October 7 2026 news").window.is_some());
}

#[test]
fn section_tag_and_home_pages_rank_below_stories_for_a_news_query() {
    let order = news(
        "latest world news",
        vec![
            hit(
                "Sanctions: Latest News, Top Stories & Analysis - POLITICO",
                "https://www.politico.com/news/sanctions",
                STORY,
            ),
            hit("Argentina", "https://www.politico.com/tag/argentina", STORY),
            hit("Home", "https://example.com/", STORY),
            hit(
                "House passes sanctions bill",
                "https://www.politico.com/news/2026/09/16/house-passes-russia-sanctions-bill-123",
                STORY,
            ),
        ],
    );
    assert_eq!(
        order[0],
        "https://www.politico.com/news/2026/09/16/house-passes-russia-sanctions-bill-123"
    );
}

#[test]
fn video_pages_rank_below_articles_for_news_unless_video_was_asked_for() {
    let list = || {
        vec![
            plain(
                "ABC World News Tonight",
                "https://www.youtube.com/watch?v=rXWqS9yNg7Y",
            ),
            plain(
                "Strikes continue overnight",
                "https://example.org/world/strikes-continue-overnight",
            ),
        ]
    };
    assert_eq!(
        news("latest world news", list())[0],
        "https://example.org/world/strikes-continue-overnight"
    );
    assert_eq!(
        news("latest world news video", list())[0],
        "https://www.youtube.com/watch?v=rXWqS9yNg7Y"
    );
}

#[test]
fn an_established_outlet_gets_a_small_prior_on_news() {
    let order = news(
        "latest world news",
        vec![
            plain(
                "Strikes continue in the north",
                "https://a.example/world/strikes-continue-overnight",
            ),
            hit(
                "Talks resume in the capital",
                "https://www.reuters.com/world/strikes-continue-overnight-2026-10-07/",
                "Negotiators returned to the table on Tuesday after a pause of several days.",
            ),
        ],
    );
    assert!(order[0].contains("reuters"));
}

#[test]
fn a_sitemap_and_a_page_of_only_chrome_rank_down() {
    let ranked = run(
        "apple support",
        vec![src(
            P::Parallel,
            vec![
                hit("Site Map", "https://apple.com/find", "- Contact Support"),
                plain("Apple Support", "https://support.apple.com/"),
            ],
        )],
    );
    assert_eq!(ranked.hits[0].url, "https://support.apple.com/");
}

#[test]
fn an_empty_snippet_takes_the_readable_one_another_provider_had() {
    let ranked = run(
        "hotline install",
        vec![
            src(
                P::Parallel,
                vec![hit(
                    "Install | Hotline",
                    "https://hotline.dev/docs/install/",
                    "Published: N/A",
                )],
            ),
            src(
                P::Keenable,
                vec![hit(
                    "Install | Hotline",
                    "https://hotline.dev/docs/install/",
                    "Run the install script and open the desk.",
                )],
            ),
        ],
    );
    assert_eq!(ranked.hits.len(), 1);
    assert_eq!(
        ranked.hits[0].snippet,
        "Run the install script and open the desk."
    );
}

#[test]
fn a_bare_ip_host_is_not_a_result() {
    let ranked = run(
        "rustc E0502",
        vec![src(
            P::Parallel,
            vec![
                hit("ovdxkvngzvchdlk", "https://142.249.174.17/x", SENTENCE),
                plain(
                    "Error codes",
                    "https://doc.rust-lang.org/error_codes/E0502.html",
                ),
            ],
        )],
    );
    assert_eq!(
        urls(&ranked),
        vec!["https://doc.rust-lang.org/error_codes/E0502.html"]
    );
}

#[test]
fn an_ordinary_modifier_does_not_raise_the_miss_warning_and_a_made_up_word_does() {
    let page = vec![src(
        P::Parallel,
        vec![hit(
            "Everest",
            "https://fr.wikipedia.org/wiki/Everest",
            "Le mont Everest culmine a 8849 metres d'altitude.",
        )],
    )];
    assert_eq!(run("Mont Everest hauteur officielle", page).note, None);
    let page = vec![src(
        P::Parallel,
        vec![hit(
            "Everest",
            "https://fr.wikipedia.org/wiki/Everest",
            "Le mont Everest culmine a 8849 metres d'altitude.",
        )],
    )];
    assert!(
        run("Mont Everest zxqvbnk", page)
            .note
            .unwrap()
            .contains("zxqvbnk")
    );
}

#[test]
fn a_number_is_met_only_as_a_whole_number() {
    let page = |text: &str| {
        vec![src(
            P::Parallel,
            vec![hit("Page", "https://x.example/p", text)],
        )]
    };
    for text in [
        "RFC 9842 defines it",
        "catalogue 98421-4-RR antibody",
        "id 198421 and 98421x",
    ] {
        let note = run("xqzflarnib 98421", page(text)).note.unwrap_or_default();
        assert!(note.contains("98421"), "{text}: {note}");
    }
    let note = run("xqzflarnib 98421", page("code 98421 listed"))
        .note
        .unwrap_or_default();
    assert!(
        note.contains("xqzflarnib") && !note.contains("98421"),
        "{note}"
    );
}

#[test]
fn a_result_with_none_of_the_distinctive_words_ranks_below_one_that_has_them() {
    let ranked = run(
        "rustc E0502",
        vec![src(
            P::Parallel,
            vec![
                plain("Rust compiler overview", "https://a.example/rustc"),
                hit(
                    "E0502",
                    "https://b.example/e0502",
                    "The E0502 error is a borrow conflict in rustc.",
                ),
            ],
        )],
    );
    assert_eq!(ranked.hits[0].url, "https://b.example/e0502");
}

#[test]
fn date_and_recency_words_are_not_the_subject() {
    let q = on("latest world news October 7 2026");
    assert_eq!(q.content_tokens(), Vec::<&str>::new());
    let ranked = rank(
        &q,
        vec![src(
            P::Parallel,
            vec![plain(
                "Talks",
                "https://a.example/world/talks-continue-tonight",
            )],
        )],
        5,
    );
    assert_eq!(ranked.note, None);
}
