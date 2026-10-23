//! The room's agent accounts: the `agentAccounts` setting, and each
//! account's folder and the setup in it. The list also names every Hotline
//! Agent credential and who it serves.
//!
//! A folder is made and a setup copied before the policy lock is taken, so a
//! slow disk does not hold up a teammate's edit; the lock is held only to
//! check the list again and write it.

use super::{now, off_the_reactor};
use crate::accounts::resolve::{self, Named};
use crate::accounts::setup::{copy_setup, nothing_to_copy, setup_view, system_default_source};
use crate::accounts::support::{AccountSupport, account_support};
use crate::accounts::{NO_ACCOUNTS, NOT_ON_THIS_DESK, login, model_provider, resolved};
use crate::contract::{
    AccountAgent, AccountCreated, AccountSetupOrigin, AccountView, AccountsList, AgentAccount,
    Command, CredentialAccount, LoginAttemptState, Persona, SetupFrom,
};
use crate::driver::HOTLINE_BACKEND_ID;
use crate::driver::acp::registry;
use crate::log::{Log, StreamId};
use crate::vault::Vault;
use crate::wire::RoomHandle;
use crate::{paths, private_dir, room};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use uuid::Uuid;

const MAX_NAME_CHARS: usize = 64;

pub(in crate::wire) async fn run(
    command: Command,
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    seat_is_desk: bool,
) -> Result<Value, String> {
    match command {
        Command::AccountsList {} => Ok(json!(list(log, room, seat_is_desk).await)),
        Command::AccountsCreate {
            backend_id,
            name,
            start_with,
            categories,
        } => {
            create(
                log,
                room,
                backend_id,
                &name,
                start_with,
                categories,
                seat_is_desk,
            )
            .await
        }
        Command::AccountsRename { account_id, name } => {
            rename(log, room, &account_id, &name, seat_is_desk).await
        }
        Command::AccountsRemove { account_id } => {
            remove(log, room, &account_id).await.map(|()| Value::Null)
        }
        Command::AccountsStatus { account_id } => room
            .accounts_status(&account_id)
            .await
            .map(|status| json!(status)),
        Command::AccountsSetupList { account_id } => setup_list(log, account_id).await,
        Command::AccountsSetupCopy {
            account_id,
            categories,
        } => setup_copy(log, room, account_id, categories).await,
        _ => Err("That is not a command about agent accounts.".into()),
    }
}

async fn list(log: &Log, room: &Arc<dyn RoomHandle>, seat_is_desk: bool) -> AccountsList {
    let agents = room
        .backends()
        .await
        .into_iter()
        .filter(|backend| backend.id != HOTLINE_BACKEND_ID)
        .map(|backend| {
            let support = account_support(&backend.id);
            AccountAgent {
                supports_accounts: support.is_some(),
                takes_email: support.is_some_and(|support| support.email_flag.is_some()),
                provider_id: support.map(|support| support.provider_id.to_string()),
                backend_id: backend.id,
                name: backend.name,
            }
        })
        .collect();
    let roster = room::roster(log);
    let accounts: Vec<AccountView> = room::agent_accounts(log)
        .into_iter()
        .map(|account| view(log, room, &roster, account, seat_is_desk))
        .collect();
    let ids: Vec<String> = accounts
        .iter()
        .map(|view| view.account.id.clone())
        .collect();
    room.accounts_refresh_stale(&ids);
    let credentials = credential_accounts(log, room, &roster);
    AccountsList {
        agents,
        accounts,
        credentials,
    }
}

fn credential_accounts(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    roster: &[Persona],
) -> Vec<CredentialAccount> {
    let events = log.load(&StreamId::Room);
    let preferred = crate::models::preferred_model(&room::settings_from_events(&events));
    let defaults = room.default_credentials();
    room.credentials()
        .into_iter()
        .map(|credential| {
            let default = defaults.get(&credential.provider_id) == Some(&credential.id);
            let used_by = roster
                .iter()
                .filter(|persona| {
                    persona.account_id.as_deref() == Some(credential.id.as_str())
                        || (default
                            && follows_default(
                                &events,
                                persona,
                                &credential.provider_id,
                                preferred.as_deref(),
                            ))
                })
                .map(|persona| persona.id.clone())
                .collect();
            CredentialAccount {
                section: Vault::section_of(&credential.provider_id, credential.credential_kind),
                used_by,
                default,
                credential,
            }
        })
        .collect()
}

/// Whether `persona`'s turns run on `provider_id`'s default: a Hotline Agent
/// teammate whose model is on that provider, the room's preferred model when
/// it names none, and whose choice is not one of that provider's credentials.
/// An id that names nothing refuses the start, so it runs on nothing.
fn follows_default(
    events: &[Value],
    persona: &Persona,
    provider_id: &str,
    preferred: Option<&str>,
) -> bool {
    let on_provider = model_provider(persona.model_id.as_deref(), preferred) == Some(provider_id);
    if persona.backend_id != HOTLINE_BACKEND_ID || !on_provider {
        return false;
    }
    match persona.account_id.as_deref() {
        None => true,
        Some(account_id) => match resolve::named(events, account_id) {
            Named::Credential(chosen) => chosen.provider_id != provider_id,
            Named::Agent(_) => true,
            Named::Nothing => false,
        },
    }
}

/// A credential a teammate is set to stays until none is. Its provider's
/// default may go: the teammates that follow it move to the next.
pub(super) fn check_unused(log: &Log, room: &Arc<dyn RoomHandle>, id: &str) -> Result<(), String> {
    let Some(credential) = room.credentials().into_iter().find(|row| row.id == id) else {
        return Ok(());
    };
    let roster = room::roster(log);
    let names: Vec<&str> = users(&roster, id)
        .map(|persona| persona.name.as_str())
        .collect();
    if names.is_empty() {
        return Ok(());
    }
    Err(in_use(&credential.label, &names))
}

fn view(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    roster: &[Persona],
    account: AgentAccount,
    seat_is_desk: bool,
) -> AccountView {
    let (status, attempt) = room.account_live(&account.id);
    AccountView {
        status,
        attempt,
        used_by: users(roster, &account.id)
            .map(|persona| persona.id.clone())
            .collect(),
        folder: paths::agent_account_dir(log.root(), &account.id)
            .map(|folder| folder.to_string_lossy().into_owned()),
        can_sign_in_here: login::can_sign_in_here(seat_is_desk),
        account,
    }
}

/// A stored choice counts whatever agent its teammate is on now, because it
/// applies again if the teammate goes back to one that has accounts.
fn users<'a>(roster: &'a [Persona], account_id: &'a str) -> impl Iterator<Item = &'a Persona> {
    roster
        .iter()
        .filter(move |persona| persona.account_id.as_deref() == Some(account_id))
}

static CREATING: Mutex<Vec<(PathBuf, String, String)>> = Mutex::new(Vec::new());

/// Holds a name from before its first check until its account is written:
/// copying a setup can take minutes, and a second create or a rename to the
/// name meanwhile would leave the first copy to be refused.
struct Creating((PathBuf, String, String));

impl Creating {
    fn reserve(root: &Path, backend_id: &str, name: &str) -> Result<Self, String> {
        let key = (
            root.to_path_buf(),
            backend_id.to_string(),
            name.to_lowercase(),
        );
        let mut creating = CREATING.lock().unwrap_or_else(PoisonError::into_inner);
        if creating.contains(&key) {
            return Err(format!(
                "An account named {name} is still being made for that agent."
            ));
        }
        creating.push(key.clone());
        Ok(Self(key))
    }
}

impl Drop for Creating {
    fn drop(&mut self) {
        CREATING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|key| *key != self.0);
    }
}

async fn create(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    backend_id: String,
    name: &str,
    start_with: SetupFrom,
    categories: Option<Vec<String>>,
    seat_is_desk: bool,
) -> Result<Value, String> {
    let support = account_support(&backend_id).ok_or(NO_ACCOUNTS)?;
    let _creating = Creating::reserve(log.root(), &backend_id, name.trim())?;
    let name = checked_name(&room::agent_accounts(log), &backend_id, name, None)?;
    // Everything that can refuse is asked before a folder exists.
    let source = match start_with {
        SetupFrom::Empty => None,
        SetupFrom::SystemDefault => {
            let source = system_default_source(support)
                .ok_or_else(|| nothing_to_copy(&agent_name(log.root(), &backend_id)))?;
            let every = || support.setup.iter().map(|item| item.id.into()).collect();
            let categories = checked_categories(support, categories.unwrap_or_else(every))?;
            Some((source, categories))
        }
    };
    let id = Uuid::new_v4().to_string();
    let (root, folder_id, folder_name) = (log.root().to_path_buf(), id.clone(), name.clone());
    // A failed copy leaves the folder for the person to look at.
    let copy = off_the_reactor(move || {
        let dir = made_folder(&root, &folder_id, &folder_name)?;
        source
            .map(|(source, categories)| copy_setup(&source, &dir, support, &categories))
            .transpose()
            .map_err(|error| not_copied(&folder_name, &error))
    })
    .await?;

    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let mut accounts = room::agent_accounts(log);
    let name = checked_name(&accounts, &backend_id, &name, None)?;
    let at = now();
    let account = AgentAccount {
        id,
        backend_id,
        name,
        created_at: at,
        login_at: None,
        setup: Some(AccountSetupOrigin {
            from: start_with,
            at,
            categories: copy
                .as_ref()
                .map_or_else(Vec::new, |report| report.copied.clone()),
        }),
    };
    accounts.push(account.clone());
    write(log, &accounts)?;
    let account = view(log, room, &room::roster(log), account, seat_is_desk);
    Ok(json!(AccountCreated { account, copy }))
}

async fn rename(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    account_id: &str,
    name: &str,
    seat_is_desk: bool,
) -> Result<Value, String> {
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let mut accounts = room::agent_accounts(log);
    let index = accounts
        .iter()
        .position(|account| account.id == account_id)
        .ok_or(NOT_ON_THIS_DESK)?;
    accounts[index].name = checked_name(
        &accounts,
        &accounts[index].backend_id,
        name,
        Some(account_id),
    )?;
    let _creating = Creating::reserve(
        log.root(),
        &accounts[index].backend_id,
        &accounts[index].name,
    )?;
    write(log, &accounts)?;
    let account = accounts.swap_remove(index);
    Ok(json!(view(
        log,
        room,
        &room::roster(log),
        account,
        seat_is_desk
    )))
}

/// The folder, and the sign-in in it, stay: removing an account only stops
/// Hotline knowing it.
async fn remove(log: &Log, room: &Arc<dyn RoomHandle>, account_id: &str) -> Result<(), String> {
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let mut accounts = room::agent_accounts(log);
    let account = accounts
        .iter()
        .find(|account| account.id == account_id)
        .ok_or(NOT_ON_THIS_DESK)?;
    let (_, attempt) = room.account_live(account_id);
    if attempt.is_some_and(|attempt| attempt.state == LoginAttemptState::Running) {
        return Err(format!(
            "Finish or cancel {}'s sign-in first.",
            account.name
        ));
    }
    let roster = room::roster(log);
    let names: Vec<&str> = users(&roster, account_id)
        .map(|persona| persona.name.as_str())
        .collect();
    if !names.is_empty() {
        return Err(in_use(&account.name, &names));
    }
    accounts.retain(|account| account.id != account_id);
    write(log, &accounts)
}

async fn setup_list(log: &Log, account_id: String) -> Result<Value, String> {
    let log = log.clone();
    off_the_reactor(move || {
        let account = resolved(&log, &account_id)?;
        let origin = room::agent_accounts(&log)
            .into_iter()
            .find(|row| row.id == account_id)
            .and_then(|row| row.setup);
        Ok(json!(setup_view(
            &account.dir,
            account.support,
            origin,
            true
        )))
    })
    .await
}

/// A copy that fails records no new origin, though the categories copied
/// before the one that failed stay copied.
async fn setup_copy(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    account_id: String,
    categories: Vec<String>,
) -> Result<Value, String> {
    if categories.is_empty() {
        return Err("Choose at least one item to copy.".into());
    }
    let (copy_log, copy_id) = (log.clone(), account_id.clone());
    let report = off_the_reactor(move || {
        let account = resolved(&copy_log, &copy_id)?;
        let categories = checked_categories(account.support, categories)?;
        let source = system_default_source(account.support)
            .ok_or_else(|| nothing_to_copy(&agent_name(copy_log.root(), &account.backend_id)))?;
        copy_setup(&source, &account.dir, account.support, &categories)
            .map_err(|error| not_copied(&account.name, &error))
    })
    .await?;

    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let mut accounts = room::agent_accounts(log);
    let account = accounts
        .iter_mut()
        .find(|account| account.id == account_id)
        .ok_or(NOT_ON_THIS_DESK)?;
    account.setup = Some(AccountSetupOrigin {
        from: SetupFrom::SystemDefault,
        at: now(),
        categories: report.copied.clone(),
    });
    write(log, &accounts)?;
    Ok(json!(report))
}

fn checked_name(
    accounts: &[AgentAccount],
    backend_id: &str,
    name: &str,
    except: Option<&str>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An account needs a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "An account name is at most {MAX_NAME_CHARS} characters."
        ));
    }
    let taken = accounts.iter().any(|account| {
        account.backend_id == backend_id
            && Some(account.id.as_str()) != except
            && account.name.to_lowercase() == name.to_lowercase()
    });
    if taken {
        return Err(format!(
            "There is already an account named {name} for that agent."
        ));
    }
    Ok(name.to_string())
}

fn checked_categories(
    support: &AccountSupport,
    categories: Vec<String>,
) -> Result<Vec<String>, String> {
    match categories
        .iter()
        .find(|id| !support.setup.iter().any(|item| item.id == id.as_str()))
    {
        Some(unknown) => Err(format!("Unknown setup item: {unknown}")),
        None => Ok(categories),
    }
}

/// The room's own reading of the setting is the one validator: a list that
/// does not read back unchanged is not written.
fn write(log: &Log, accounts: &[AgentAccount]) -> Result<(), String> {
    if hotline_room::agent_accounts::normalize_accounts(&json!(accounts)) != accounts {
        return Err("That change would leave an account the room cannot read.".into());
    }
    room::write_agent_accounts(log, accounts)
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}

fn made_folder(root: &Path, id: &str, name: &str) -> Result<PathBuf, String> {
    let not_made = |error: std::io::Error| format!("{name}'s folder could not be made: {error}");
    let folder = paths::agent_account_dir(root, id).expect("a new account's uuid names a folder");
    private_dir::make_private_directory(&paths::agent_accounts_dir(root)).map_err(not_made)?;
    private_dir::make_private_directory(&folder).map_err(not_made)?;
    private_dir::checked_account_dir(root, id).map_err(not_made)
}

fn not_copied(name: &str, error: &std::io::Error) -> String {
    format!("The setup could not be copied into {name}'s folder: {error}")
}

fn agent_name(root: &Path, backend_id: &str) -> String {
    registry::name_of(root, backend_id).unwrap_or_else(|| backend_id.to_string())
}

/// Why a removal is refused: "Work can't be removed while Ada uses it." for
/// one teammate, "... while Ada and Bob use it." for more.
fn in_use(name: &str, teammates: &[&str]) -> String {
    let verb = if teammates.len() == 1 { "uses" } else { "use" };
    format!(
        "{name} can't be removed while {} {verb} it.",
        joined(teammates)
    )
}

/// "A", "A and B", "A, B and C".
fn joined(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
