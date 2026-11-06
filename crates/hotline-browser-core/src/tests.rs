use super::*;

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

#[test]
fn local_addresses_are_this_machine_and_the_local_network() {
    for local in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.9",
        "192.168.1.20",
        "169.254.0.1",
        "100.64.0.1",
        "0.0.0.0",
        "::1",
        "fd00::1",
        "fe80::1",
        "::ffff:127.0.0.1",
    ] {
        assert!(is_local_ip(local.parse().unwrap()), "{local}");
    }
    for public in ["8.8.8.8", "1.1.1.1", "100.128.0.1", "2606:4700::1111"] {
        assert!(!is_local_ip(public.parse().unwrap()), "{public}");
    }
}

#[test]
fn a_link_names_a_local_address_by_localhost_dot_local_or_an_ip() {
    assert!(names_local(&url("http://localhost:2322/")));
    assert!(names_local(&url("http://app.localhost/")));
    assert!(names_local(&url("http://printer.local/")));
    assert!(names_local(&url("http://192.168.1.20/")));
    assert!(names_local(&url("http://[::1]:8080/")));
    assert!(!names_local(&url("https://example.com/")));
}

#[tokio::test]
async fn a_name_that_resolves_to_this_machine_reaches_a_local_address() {
    // `localhost.` with its trailing dot resolves through the system,
    // and the IP literal never asks DNS at all.
    assert!(reaches_local(&url("http://127.0.0.1:2322/")).await);
    assert!(reaches_local(&url("http://localhost.:2322/")).await);
}

#[test]
fn an_origin_key_is_the_host_and_port_the_card_shows() {
    assert_eq!(
        origin_key(&url("http://localhost:2322/a")).as_deref(),
        Some("localhost:2322")
    );
    assert_eq!(
        origin_key(&url("http://192.168.1.20/")).as_deref(),
        Some("192.168.1.20:80")
    );
    assert_eq!(
        origin_key(&url("http://[::1]:8080/")).as_deref(),
        Some("[::1]:8080")
    );
}

#[test]
fn the_gate_admits_every_web_page_but_the_desks_own_ports() {
    let gate = Gate::default();
    gate.never.write().unwrap().insert(4000);
    assert!(gate.admits(&url("https://example.com/")));
    assert!(gate.admits(&url("about:blank")));
    assert!(gate.admits(&url("http://localhost:2322/")));
    assert!(gate.admits(&url("https://pm.dfam.example:8443/sso")));
    assert!(gate.admits(&url("http://192.168.1.20/")));
    assert!(!gate.admits(&url("http://localhost:4000/")));
    assert!(!gate.admits(&url("http://127.0.0.1:4000/door")));
    assert!(!gate.admits(&url("file:///etc/passwd")));
    assert!(!gate.admits(&url("tauri://localhost/")));
    assert!(
        !gate.admits(&url("about:srcdoc")) && !gate.admits(&url("data:text/html,hi")),
        "the top of the window is a web page or the blank one"
    );
}

#[test]
fn a_frame_loads_what_a_browser_would_but_files_the_shell_and_the_desk() {
    let gate = Gate::default();
    gate.never.write().unwrap().insert(4000);
    for framed in [
        "about:blank",
        "about:srcdoc",
        "data:text/html,<p>hi</p>",
        "blob:https://pay.example/1b2c",
        "https://accounts.example/embed",
        "http://192.168.1.20:8080/widget",
    ] {
        assert!(gate.admits_frame(&url(framed)), "{framed}");
    }
    for refused in [
        "file:///etc/passwd",
        "tauri://localhost/index.html",
        "ipc://localhost/plugin",
        "about:config",
        "http://localhost:4000/",
        "http://127.0.0.1.nip.io:4000/",
    ] {
        assert!(!gate.admits_frame(&url(refused)), "{refused}");
    }
}

#[test]
fn a_desk_port_is_refused_by_any_name_and_any_local_address_but_not_a_public_one() {
    let gate = Gate::default();
    gate.never.write().unwrap().insert(4000);
    // A name is the desk's on one of its ports whatever it says: it may
    // resolve to this machine (nip.io answers the address it names), or
    // answer differently at the load than at a check.
    for own in [
        "http://127.0.0.1.nip.io:4000/",
        "http://localhost.:4000/",
        "https://example.com:4000/",
        "http://[::ffff:127.0.0.1]:4000/",
        "http://10.0.0.5:4000/",
        "http://2130706433:4000/",
    ] {
        assert!(gate.own(&url(own)), "{own}");
        assert!(!gate.admits(&url(own)), "{own}");
    }
    assert!(
        gate.admits(&url("http://93.184.216.34:4000/")),
        "a public address on that port is another server"
    );
    assert!(gate.admits(&url("http://127.0.0.1.nip.io:2322/")));
}

#[test]
fn every_listener_of_the_process_and_every_running_computer_is_the_desks() {
    let gate = Gate::default();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let page = url(&format!("http://127.0.0.1:{port}/mcp"));
    assert!(gate.admits(&page), "not held yet");
    let held = hotline_support::listening::hold(port);
    assert!(!gate.admits(&page), "a listener held is refused");
    drop(held);
    assert!(gate.admits(&page), "and opens once it is let go");

    let live = std::sync::Arc::new(RwLock::new(Vec::<u16>::new()));
    {
        let live = live.clone();
        gate.watch_computers(move || live.read().unwrap().clone());
    }
    // Below every platform's ephemeral range, so no listener another
    // test holds can be on it.
    let computer = url("http://127.0.0.1:9417/mcp");
    assert!(gate.admits(&computer));
    live.write().unwrap().push(9417);
    assert!(
        !gate.admits(&computer),
        "a computer that came up after install"
    );
    live.write().unwrap().clear();
    assert!(gate.admits(&computer));
}

#[test]
fn a_forgotten_teammates_answers_are_gone_from_the_run_and_the_room() {
    let dir = tempfile::tempdir().unwrap();
    let log = Log::open(dir.path());
    let gate = Gate::default();
    gate.allow_always(&log, "kit", "localhost:2322").unwrap();
    gate.allow("kit", "localhost:5173");
    gate.allow_always(&log, "ada", "localhost:2322").unwrap();
    gate.forget(&log, "kit");
    assert!(!gate.granted("kit", "localhost:2322"));
    assert!(!gate.granted("kit", "localhost:5173"));
    assert!(gate.granted("ada", "localhost:2322"), "another's stand");
    let next = Gate::default();
    next.load(&log, &[]);
    assert!(
        !next.granted("kit", "localhost:2322"),
        "struck from the room"
    );
    assert!(next.granted("ada", "localhost:2322"));
}

#[test]
fn a_certificate_fingerprint_is_its_sha_256() {
    assert_eq!(
        certificate_fingerprint(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

fn input(kind: &str, autocomplete: &str, words: &str) -> Field {
    Field {
        tag: "input".into(),
        kind: kind.into(),
        autocomplete: autocomplete.into(),
        words: words.into(),
        ..Field::default()
    }
}

#[test]
fn a_login_part_goes_only_into_a_field_made_for_it() {
    use LoginPart::{Code, Password, Username};
    let takes = |field: &Field| {
        [Username, Password, Code]
            .into_iter()
            .filter(|part| field.takes(*part))
            .collect::<Vec<_>>()
    };
    let numeric = |kind: &str, words: &str| Field {
        inputmode: "numeric".into(),
        ..input(kind, "", words)
    };
    let in_form = |field: Field| Field {
        with_password: true,
        ..field
    };
    for (field, parts, why) in [
        (
            input("password", "current-password", "password"),
            vec![Password, Code],
            "a password field",
        ),
        (input("password", "", ""), vec![Password, Code], "any one"),
        (
            input("text", "one-time-code", ""),
            vec![Code],
            "autocomplete says one-time code",
        ),
        (
            input("tel", "section-mfa one-time-code", ""),
            vec![Code],
            "past a section",
        ),
        (numeric("text", "app_otp"), vec![Code], "numeric, named otp"),
        (
            numeric("text", "user[otp_attempt]"),
            vec![Code],
            "a word starting with otp",
        ),
        (
            numeric("text", "verification code"),
            vec![Code],
            "labelled a code",
        ),
        (input("tel", "", "totpPin"), vec![Code], "tel, named totp"),
        (
            numeric("text", "zip code"),
            vec![],
            "a postal code is not one",
        ),
        (
            Field {
                inputmode: "numeric".into(),
                ..input("text", "postal-code", "code")
            },
            vec![],
            "nor a field whose autocomplete is another purpose",
        ),
        (numeric("text", "amount"), vec![], "digits alone"),
        (
            input("text", "", "app_otp"),
            vec![],
            "a code's name on a field that takes any text",
        ),
        (
            input("text", "username", "login_field"),
            vec![Username],
            "autocomplete username",
        ),
        (
            input("text", "username webauthn", ""),
            vec![Username],
            "past webauthn",
        ),
        (
            input("email", "", "email"),
            vec![Username],
            "an email field",
        ),
        (
            input("tel", "username", "phone"),
            vec![Username],
            "a phone number as the username",
        ),
        (
            in_form(input("text", "", "login")),
            vec![Username],
            "a text field in a form with a password field",
        ),
        (
            input("text", "", "login"),
            vec![],
            "a text field in no sign-in form",
        ),
        (
            in_form(input("text", "street-address", "address")),
            vec![],
            "a field for something else in that form",
        ),
        (in_form(input("search", "", "")), vec![], "a search field"),
        (
            in_form(input("text", "username", "Search or jump to…")),
            vec![],
            "worded like search",
        ),
        (in_form(input("text", "", "q")), vec![], "named q"),
        (
            Field {
                in_search: true,
                ..input("password", "", "")
            },
            vec![],
            "in a search landmark",
        ),
        (
            Field {
                role: "searchbox".into(),
                ..input("email", "", "")
            },
            vec![],
            "a searchbox",
        ),
        (
            Field {
                tag: "textarea".into(),
                ..in_form(input("", "username", ""))
            },
            vec![],
            "a text area",
        ),
        (
            Field {
                tag: "editable".into(),
                ..in_form(input("", "", ""))
            },
            vec![],
            "an editable part of the page",
        ),
    ] {
        assert_eq!(takes(&field), parts, "{why}: {field:?}");
    }
}

#[test]
fn a_secrets_fill_never_shows_its_text_in_debug() {
    let act = Act::FillSecret {
        target: "e3".into(),
        text: "hunter22".into(),
        part: LoginPart::Password,
        field: input("password", "", ""),
    };
    assert!(!format!("{act:?}").contains("hunter22"));
}

#[test]
fn a_fill_never_shows_its_text_in_debug() {
    let act = Act::Fill {
        target: "e3".into(),
        text: "hunter2".into(),
    };
    assert!(!format!("{act:?}").contains("hunter2"));
}

/// An answer taken back stays taken back across a restart, and one given
/// again after that stands again.
#[test]
fn an_always_allow_taken_back_is_gone_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let log = Log::open(dir.path());
    let gate = Gate::default();
    gate.allow_always(&log, "kit", "localhost:2322").unwrap();
    gate.allow_always(&log, "kit", "localhost:5173").unwrap();
    assert_eq!(
        gate.always(&log, "kit"),
        ["localhost:2322", "localhost:5173"]
    );
    gate.disallow(&log, "kit", "localhost:2322").unwrap();
    assert!(!gate.granted("kit", "localhost:2322"));
    let restarted = Gate::default();
    restarted.load(&log, &[]);
    assert!(
        !restarted.granted("kit", "localhost:2322"),
        "struck stays struck"
    );
    assert!(restarted.granted("kit", "localhost:5173"));
    restarted
        .allow_always(&log, "kit", "localhost:2322")
        .unwrap();
    let again = Gate::default();
    again.load(&log, &[]);
    assert!(again.granted("kit", "localhost:2322"), "given again stands");
}

#[test]
fn always_allow_is_kept_in_the_room_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let log = Log::open(dir.path());
    Gate::default()
        .allow_always(&log, "kit", "localhost:2322")
        .unwrap();
    let gate = Gate::default();
    gate.load(&log, &[7000]);
    assert!(gate.granted("kit", "localhost:2322"));
    assert!(gate.never(7000));
    assert!(gate.never(OLLAMA_PORT));
}
