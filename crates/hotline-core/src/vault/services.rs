//! The services a key can be "Used with", from `services.json`: the one
//! list the desk resolves a choice through, the window and the phone offer,
//! and a protected computer is handed inside a key's `use`.
//!
//! A prefix only suggests a service; the person confirms it. Other is the
//! person's own host, with the `Authorization` header and no refuse list.

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
    Ok(VariableUse {
        origins,
        header: service.header.clone(),
        schemes: service.schemes.clone(),
        refuse: service.refuse.clone(),
    })
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
        assert!(anthropic.schemes.is_empty());
    }

    #[test]
    fn other_is_the_persons_https_host_with_no_refuse_list() {
        let other = resolve(&choice("other", Some("api.example.com"))).unwrap();
        assert_eq!(other.origins, ["https://api.example.com"]);
        assert_eq!(other.header, "Authorization");
        assert!(other.refuse.is_empty());
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
