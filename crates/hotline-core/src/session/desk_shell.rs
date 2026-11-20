//! What a teammate's desk shell is handed of its secret grants.
//!
//! A grant reaches the desk shell only when it names it (`on` holds
//! `desk`): every grant made before the desk shell took secrets was made
//! for a computer, and stays there. Of such a grant:
//!
//! - a **readable** variable is put in the environment of each command the
//!   teammate runs, and of an ACP harness when it starts, as the person
//!   confirmed when they gave it;
//! - a **protected** key (a variable Used with a service) reaches only a
//!   confined Hotline Agent's shell, as a stand-in made for that one
//!   command, with `HTTPS_PROXY` naming a proxy user of its own on the
//!   desk's key proxy (`crate::keyproxy`), which adds the key on the way to
//!   the service. A teammate that runs as the person never gets one: it
//!   could read the desk's memory or files the proxy stands in for.
//!
//! What is handed is read from the vault when the teammate's session starts
//! and again whenever a grant or a stored secret changes, never per command.
//! A change takes a revoked key's stand-ins away at once, in commands still
//! running too; a readable value already in a running command's environment
//! stays there, as it would on any machine.

use super::*;
use crate::contract::{GrantPlace, Reach, SecretGrant};
use crate::keyproxy::{Binding, KeyProxy, StandIn, Use};
use hotline_tools::{RunEnv, ShellSecrets};

/// A teammate's readable variables, `(NAME, value)`.
type Readable = Vec<(String, String)>;

/// Where a run's proxied traffic does not go: the machine's own servers.
const NO_PROXY: &str = "localhost,127.0.0.1,::1";

#[derive(Default)]
pub(crate) struct DeskShell {
    /// Started the first time a teammate is given a protected key for its
    /// desk shell, on a loopback port of its own.
    proxy: Mutex<Option<Arc<Proxy>>>,
    /// Each teammate's readable variables for its desk shell, by teammate.
    readable: Mutex<HashMap<String, Vec<(String, String)>>>,
}

struct Proxy {
    keys: KeyProxy,
    port: u16,
}

/// A run's proxy user, ended when the command is.
struct RunUser {
    keys: KeyProxy,
    user: String,
}

impl Drop for RunUser {
    fn drop(&mut self) {
        self.keys.end(&self.user);
    }
}

impl DeskShell {
    fn running(&self) -> Option<Arc<Proxy>> {
        lock(&self.proxy).clone()
    }

    fn started(&self) -> Result<Arc<Proxy>, String> {
        let mut proxy = lock(&self.proxy);
        if let Some(running) = proxy.as_ref() {
            return Ok(running.clone());
        }
        let keys = KeyProxy::new()?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("The key proxy could not listen: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "The key proxy needs the desk's runtime.".to_owned())?;
        runtime.spawn(keys.clone().serve(listener));
        let running = Arc::new(Proxy { keys, port });
        *proxy = Some(running.clone());
        Ok(running)
    }

    /// Replaces what `teammate`'s desk shell is handed. Nothing is how it is
    /// all taken back.
    fn hand(
        &self,
        teammate: &str,
        readable: Vec<(String, String)>,
        protected: Vec<Binding>,
    ) -> Result<(), String> {
        let mut readable_now = lock(&self.readable);
        if readable.is_empty() {
            readable_now.remove(teammate);
        } else {
            readable_now.insert(teammate.to_owned(), readable);
        }
        drop(readable_now);
        match (self.running(), protected.is_empty()) {
            (None, true) => Ok(()),
            (Some(proxy), true) => proxy.keys.deliver(teammate, Vec::new()),
            (_, false) => {
                let proxy = self.started()?;
                if let Err(error) = proxy.keys.deliver(teammate, protected) {
                    // All or nothing: a set the proxy refused leaves none.
                    let _ = proxy.keys.deliver(teammate, Vec::new());
                    return Err(error);
                }
                Ok(())
            }
        }
    }

    /// The readable variables `teammate` is handed on its desk.
    fn readable(&self, teammate: &str) -> Vec<(String, String)> {
        lock(&self.readable)
            .get(teammate)
            .cloned()
            .unwrap_or_default()
    }

    /// What one command of `teammate`'s is handed: its readable variables,
    /// and in a confined shell a stand-in for each protected key with a
    /// proxy user of its own for the command's life.
    pub(crate) fn run_env(&self, teammate: &str, reach: Reach) -> RunEnv {
        let mut env = RunEnv {
            vars: self.readable(teammate),
            ..RunEnv::default()
        };
        let Some(proxy) = self.running() else {
            return env;
        };
        if reach != Reach::Workspace || !proxy.keys.holds(teammate) {
            return env;
        }
        let access = proxy.keys.begin(teammate);
        let url = format!(
            "http://{}:{}@127.0.0.1:{}",
            access.user, access.password, proxy.port
        );
        env.vars.extend(access.stand_ins);
        for name in ["HTTPS_PROXY", "https_proxy"] {
            env.vars.push((name.to_owned(), url.clone()));
        }
        for name in ["NO_PROXY", "no_proxy"] {
            env.vars.push((name.to_owned(), NO_PROXY.to_owned()));
        }
        // Node's own fetch reads HTTPS_PROXY only when told to.
        env.vars
            .push(("NODE_USE_ENV_PROXY".to_owned(), "1".to_owned()));
        env.ca_pem = Some(proxy.keys.ca_pem().to_owned());
        env.held = Some(Box::new(RunUser {
            keys: proxy.keys.clone(),
            user: access.user,
        }));
        env
    }
}

/// A teammate's shell grants, looked up as each command starts.
struct TeammateShell {
    room: Weak<Room>,
    persona_id: String,
}

impl ShellSecrets for TeammateShell {
    fn for_run(&self, reach: Reach) -> RunEnv {
        self.room
            .upgrade()
            .map(|room| room.desk_shell.run_env(&self.persona_id, reach))
            .unwrap_or_default()
    }
}

pub(crate) fn for_teammate(room: Weak<Room>, persona_id: String) -> Arc<dyn ShellSecrets> {
    Arc::new(TeammateShell { room, persona_id })
}

impl Room {
    /// Reads from the vault what `persona`'s desk shell is granted now and
    /// hands it over, taking back what it no longer is. A grant that cannot
    /// be read hands nothing, rather than an old value.
    pub(crate) fn refresh_desk_shell(&self, persona: &Persona) {
        let notice = |text: String| eprintln!("{}: {text}", persona.name);
        let grants: Vec<SecretGrant> = persona
            .grants()
            .into_iter()
            .filter(|grant| grant.reaches(GrantPlace::Desk))
            .collect();
        let (readable, protected) = match self.desk_shell_set(persona, &grants) {
            Ok(set) => set,
            Err(error) => {
                notice(error);
                (Vec::new(), Vec::new())
            }
        };
        if let Err(error) = self.desk_shell.hand(&persona.id, readable, protected) {
            notice(format!(
                "{}'s desk shell has none of its keys used with a service: {error}",
                persona.name
            ));
        }
    }

    fn desk_shell_set(
        &self,
        persona: &Persona,
        grants: &[SecretGrant],
    ) -> Result<(Readable, Vec<Binding>), String> {
        if grants.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let Some(vault) = &self.vault else {
            return Ok((Vec::new(), Vec::new()));
        };
        let names: Vec<String> = grants.iter().map(|grant| grant.name.clone()).collect();
        let (values, _missing) = vault.shared_secret_values(&names).map_err(|error| {
            format!("The secrets granted to its desk shell could not be read: {error}")
        })?;
        let as_person = super::grants::runs_as_the_person(persona);
        let mut readable = Vec::new();
        let mut protected = Vec::new();
        for grant in grants {
            let Some(crate::vault::StoredSecret::Variable { value, r#use }) =
                values.get(&grant.name)
            else {
                // A login or a passkey is used by a browser, never a shell.
                continue;
            };
            if grant.readable {
                readable.push((grant.name.clone(), value.clone()));
            } else if let (false, Some(usage)) = (as_person, r#use) {
                protected.push(Binding {
                    name: grant.name.clone(),
                    value: value.clone(),
                    use_: Use {
                        origins: usage.origins.clone(),
                        header: usage.header.clone(),
                        schemes: usage.schemes.clone(),
                        stand_in: Some(StandIn {
                            prefix: crate::vault::stand_in_prefix(value).to_owned(),
                            length: value.chars().count(),
                        }),
                        refuse: Some(usage.refuse.clone()),
                    },
                });
            }
        }
        Ok((readable, protected))
    }

    /// Every teammate's desk shell, handed what it is granted now: after a
    /// stored secret changed, or a grant did.
    pub(crate) fn refresh_desk_shells(&self) {
        for persona in room::roster(&self.log) {
            self.refresh_desk_shell(&persona);
        }
    }

    /// The readable variables `persona_id`'s desk shell is granted.
    pub(crate) fn desk_shell_readable(&self, persona_id: &str) -> Vec<(String, String)> {
        self.desk_shell.readable(persona_id)
    }

    /// A deleted teammate's desk shell holds nothing.
    pub(super) fn forget_desk_shell(&self, persona_id: &str) {
        let _ = self.desk_shell.hand(persona_id, Vec::new(), Vec::new());
    }
}

#[cfg(all(test, target_os = "macos"))]
impl DeskShell {
    /// A test's proxy, already serving on `port`, in place of the one this
    /// would start.
    fn install(&self, keys: KeyProxy, port: u16) {
        *lock(&self.proxy) = Some(Arc::new(Proxy { keys, port }));
    }
}

#[cfg(test)]
mod tests;
