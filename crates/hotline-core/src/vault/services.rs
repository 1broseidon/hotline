//! The services a key can be "Used with", from `services.json`: the one
//! list the desk resolves a choice through, the window and the phone offer,
//! and a protected computer is handed inside a key's `use`.
//!
//! A prefix only suggests a service; the person confirms it. Other is the
//! person's own host, with the `Authorization` header, refused only what a
//! named service on that host is refused.
//!
//! The vault keeps the choice, and [`current`] reads what it means from this
//! list each time a key is listed or handed on: a refuse list made longer
//! here reaches every key stored before it.

use crate::contract::{SecretService, UsedWith, VariableUse};
use std::io;
use std::sync::LazyLock;

static SERVICES: LazyLock<Vec<SecretService>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("services.json")).expect("services.json is the list it is")
});

/// The id of the entry whose origins the person names.
const OTHER: &str = "other";

pub fn services() -> &'static [SecretService] {
    &SERVICES
}

/// What a "Used with" choice is handed to a computer as, or why it is not
/// one: a service this list does not know, or Other without an https host.
pub fn resolve(choice: &UsedWith) -> io::Result<VariableUse> {
    let refuse = |text: String| io::Error::new(io::ErrorKind::InvalidInput, text);
    let service = services()
        .iter()
        .find(|service| service.id == choice.service)
        .ok_or_else(|| {
            refuse(format!(
                "{} is not a service Hotline knows.",
                choice.service
            ))
        })?;
    let origins = if service.id == OTHER {
        let host = choice
            .host
            .as_deref()
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .ok_or_else(|| refuse("Other needs the host the key is sent to.".to_owned()))?;
        let site = if host.contains("://") {
            host.to_owned()
        } else {
            format!("https://{host}")
        };
        let origin = super::check_site(&site)?;
        if !origin.starts_with("https://") {
            return Err(refuse(format!(
                "A key is only added to https requests, not {origin}."
            )));
        }
        vec![origin]
    } else {
        service.origins.clone()
    };
    // A named service's host chosen as Other is still that service: what it
    // mints a key with is refused the same.
    let refuse = if service.id == OTHER {
        services()
            .iter()
            .filter(|named| named.origins.iter().any(|origin| origins.contains(origin)))
            .flat_map(|named| named.refuse.iter().cloned())
            .collect()
    } else {
        service.refuse.clone()
    };
    Ok(VariableUse {
        service: Some(service.id.clone()),
        origins,
        header: service.header.clone(),
        schemes: service.schemes.clone(),
        refuse,
    })
}

/// What a key stored "Used with" a service means now: that service's entry
/// in this list, found by its id, with the person's host for Other. Nothing
/// else the record kept is read, so a refuse list is never the one copied
/// when the key was stored. A record from before ids were kept is named by
/// its origins, as the window reads it: the named service with exactly
/// those, else Other on its first. None for a service this list no longer
/// has, which leaves the key with no "Used with": it waits on the desk
/// rather than go anywhere unprotected.
pub fn current(stored: &VariableUse) -> Option<VariableUse> {
    let service = match &stored.service {
        Some(id) => id.clone(),
        None => services()
            .iter()
            .find(|named| {
                named.id != OTHER
                    && named.origins.len() == stored.origins.len()
                    && named
                        .origins
                        .iter()
                        .all(|origin| stored.origins.contains(origin))
            })
            .map_or_else(|| OTHER.to_owned(), |named| named.id.clone()),
    };
    let host = if service == OTHER {
        Some(stored.origins.first()?.clone())
    } else {
        None
    };
    resolve(&UsedWith { service, host }).ok()
}

/// The start a key's stand-in keeps, so a program that checks the shape of
/// its token still runs: the longest known prefix the value has, or none.
pub fn stand_in_prefix(value: &str) -> &'static str {
    services()
        .iter()
        .flat_map(|service| service.prefixes.iter())
        .filter(|prefix| value.starts_with(prefix.as_str()))
        .max_by_key(|prefix| prefix.len())
        .map_or("", String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(service: &str, host: Option<&str>) -> UsedWith {
        UsedWith {
            service: service.into(),
            host: host.map(str::to_owned),
        }
    }

    #[test]
    fn the_list_is_github_openai_anthropic_and_other() {
        let ids: Vec<&str> = services().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["github", "openai", "anthropic", "other"]);
        for service in services() {
            for origin in &service.origins {
                assert_eq!(super::super::check_site(origin).unwrap(), *origin);
            }
        }
    }

    #[test]
    fn a_service_resolves_to_its_origins_header_and_refuse_list() {
        let github = resolve(&choice("github", None)).unwrap();
        assert!(
            github
                .origins
                .contains(&"https://api.github.com".to_owned())
        );
        assert_eq!(github.header, "Authorization");
        assert!(github.refuse.contains(&"/authorizations".to_owned()));
        let anthropic = resolve(&choice("anthropic", None)).unwrap();
        assert_eq!(anthropic.header, "x-api-key");
        assert_eq!(anthropic.schemes, [] as [String; 0]);
    }

    /// Every route the computer refuses on its own is in the list the desk
    /// sends in its place, for each named service on that origin.
    #[test]
    fn the_desks_lists_cover_the_computers_built_in_one() {
        let built_in = include_str!("computer_refused.txt");
        let mut checked = 0;
        for line in built_in.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (origin, route) = line.split_once(' ').expect("origin, then route");
            let on_origin: Vec<&SecretService> = services()
                .iter()
                .filter(|service| service.origins.iter().any(|named| named == origin))
                .collect();
            assert!(!on_origin.is_empty(), "no service is on {origin}");
            for service in on_origin {
                assert!(
                    service.refuse.iter().any(|listed| listed == route),
                    "{}'s list misses the computer's {route}",
                    service.id
                );
            }
            checked += 1;
        }
        assert_eq!(checked, 6);
    }

    /// The routes a job could add a credential of its own through, which
    /// the computer's own list misses.
    #[test]
    fn the_lists_refuse_adding_a_credential_and_reading_secrets() {
        let refuses = |id: &str, route: &str| {
            resolve(&choice(id, None))
                .unwrap()
                .refuse
                .contains(&route.to_owned())
        };
        for route in [
            "/user/keys",
            "/user/gpg_keys",
            "/user/ssh_signing_keys",
            "/repos/*/*/keys",
            "/applications/*/token",
            "/repos/*/*/actions/secrets",
            "/repos/*/*/actions/runners/registration-token",
            "/orgs/*/actions/runners/generate-jitconfig",
        ] {
            assert!(refuses("github", route), "{route}");
        }
        for route in [
            "/v1/organization/projects/*/api_keys",
            "/v1/organization/projects/*/service_accounts",
            "/v1/realtime/client_secrets",
        ] {
            assert!(refuses("openai", route), "{route}");
        }
        assert!(refuses("anthropic", "/v1/organizations/invites"));
    }

    /// A computer reads at most 64 routes a key, each a path of plain
    /// characters (hotline-computer `keyproxy::policy::parse`); one it
    /// cannot read refuses the whole set.
    #[test]
    fn every_list_is_one_a_computer_takes() {
        for service in services() {
            assert!(service.refuse.len() <= 64, "{}", service.id);
            for route in &service.refuse {
                assert!(route.starts_with('/') && route.len() <= 256, "{route}");
                assert!(
                    route
                        .bytes()
                        .all(|b| b.is_ascii_graphic() && !b"?#%\\".contains(&b)),
                    "{route}"
                );
            }
        }
    }

    /// A key stored before ids were kept, with the short list of its day,
    /// is handed today's list; one stored with an id, whatever it kept.
    #[test]
    fn a_stored_use_is_read_as_its_service_is_now() {
        let github = resolve(&choice("github", None)).unwrap();
        let mut stale = VariableUse {
            service: None,
            refuse: vec!["/authorizations".to_owned()],
            ..github.clone()
        };
        assert_eq!(current(&stale), Some(github.clone()));
        stale.service = Some("github".to_owned());
        stale.origins = vec!["https://api.github.com".to_owned()];
        assert_eq!(current(&stale), Some(github));

        let other = VariableUse {
            service: None,
            origins: vec!["https://api.example.com".to_owned()],
            header: "Authorization".to_owned(),
            schemes: vec!["Bearer".to_owned()],
            refuse: Vec::new(),
        };
        let read = current(&other).unwrap();
        assert_eq!(read.service.as_deref(), Some("other"));
        assert_eq!(read.origins, ["https://api.example.com"]);

        let gone = VariableUse {
            service: Some("stripe".to_owned()),
            ..other
        };
        assert_eq!(current(&gone), None, "a service this build lacks");
    }

    #[test]
    fn other_on_a_named_services_host_is_refused_what_that_service_is() {
        let other = resolve(&choice("other", Some("api.openai.com"))).unwrap();
        assert_eq!(
            other.refuse,
            resolve(&choice("openai", None)).unwrap().refuse
        );
    }

    #[test]
    fn other_is_the_persons_https_host_with_no_refuse_list() {
        let other = resolve(&choice("other", Some("api.example.com"))).unwrap();
        assert_eq!(other.origins, ["https://api.example.com"]);
        assert_eq!(other.header, "Authorization");
        assert_eq!(other.refuse, [] as [String; 0]);
        assert!(resolve(&choice("other", None)).is_err());
        assert!(resolve(&choice("other", Some("http://localhost:8080"))).is_err());
        assert!(resolve(&choice("other", Some("https://api.example.com/v1"))).is_err());
        assert!(resolve(&choice("stripe", None)).is_err());
    }

    #[test]
    fn the_longest_prefix_wins() {
        assert_eq!(stand_in_prefix("sk-ant-api03-abcdef"), "sk-ant-");
        assert_eq!(stand_in_prefix("sk-proj-abcdef"), "sk-proj-");
        assert_eq!(stand_in_prefix("github_pat_11AAAA"), "github_pat_");
        assert_eq!(stand_in_prefix("postgres://user:pw@host"), "");
    }
}
