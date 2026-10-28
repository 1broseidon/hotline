//! The lead adding a teammate the person asked for: one core-owned operation,
//! raised as a card and written only by the person's yes.

use super::asking::{Approves, Question};
use super::*;
use crate::contract::PersonaDraft;
use crate::driver::HOTLINE_BACKEND_ID;
use crate::wire::commands::persona::{add_persona, drafted_persona};

/// The longest card the person reads whole: [`Room::ask_card`] clips there,
/// and a teammate approved from a clipped card is not one the person saw.
const CARD_MAX: usize = 500;

/// All a lead may say about the teammate it asks for. Reach, extra folders,
/// MCP servers, background work and a computer have no field here, so the new
/// teammate starts at the New teammate pane's narrowest defaults and only the
/// person widens it.
pub(crate) struct NewTeammate {
    pub name: String,
    pub goal: String,
    /// An agent's id or the name `list_teammates` shows; absent is the room's
    /// default for new teammates.
    pub agent: Option<String>,
    pub model: Option<String>,
    /// The working directory; absent is a fresh workspace of its own.
    pub folder: Option<String>,
}

impl Room {
    /// Whether this teammate leads the desk now. Asked when the tools are
    /// listed, when the tool is called and again when the person answers,
    /// because the person can move the role at any time.
    pub(crate) fn is_lead(&self, persona_id: &str) -> bool {
        crate::room::team_lead(&self.log).as_deref() == Some(persona_id)
    }

    fn check_lead(&self, persona_id: &str) -> Result<(), String> {
        if self.is_lead(persona_id) {
            Ok(())
        } else {
            Err("Only the team lead adds teammates, and you are not the lead now.".into())
        }
    }

    /// Raises the card showing the teammate exactly as it would be made, the
    /// room's defaults filled in, and returns at once. Nothing is written
    /// until the person says yes (see [`Room::settle_new_teammate`]).
    pub(crate) async fn ask_new_teammate(
        &self,
        persona_id: &str,
        asked: NewTeammate,
        home: &ThreadId,
        capability: CapabilityLease,
    ) -> Result<String, String> {
        capability.check()?;
        self.check_lead(persona_id)?;
        let name = asked.name.trim().to_string();
        if name.is_empty() {
            return Err("add_teammate needs a name for the new teammate.".into());
        }
        let goal = asked.goal.trim().to_string();
        if goal.is_empty() {
            return Err("add_teammate needs a goal: what the new teammate is for.".into());
        }
        if let Some(folder) = &asked.folder
            && !Path::new(folder.trim()).is_absolute()
        {
            return Err("The folder must be an absolute path.".into());
        }
        let backend_id = match asked.agent {
            Some(agent) => Some(self.agent_id(agent.trim()).await?),
            None => None,
        };
        let draft = PersonaDraft {
            name: name.clone(),
            goal: Some(goal),
            team: None,
            backend_id,
            cwd: asked.folder,
            reach: None,
            folders: None,
            model_id: asked.model,
            effort_id: None,
            computer: None,
            background_work: None,
        };
        let persona = drafted_persona(&self.log, new_id(), draft)?;
        let agent = if persona.backend_id == HOTLINE_BACKEND_ID {
            "Hotline Agent".to_string()
        } else {
            acp::registry::name_of(self.log.root(), &persona.backend_id)
                .unwrap_or_else(|| persona.backend_id.clone())
        };
        let model = persona
            .model_id
            .as_deref()
            .unwrap_or("the agent's own default");
        let reason = format!(
            "Add {name} as a teammate? Goal: {}. Agent: {agent}, model {model}. Folder: {}.",
            persona.goal.trim_end_matches('.'),
            persona.cwd,
        );
        if reason.chars().count() > CARD_MAX {
            return Err("That card would be too long to show whole: shorten the goal.".into());
        }
        let asked = self.ask_card(
            persona_id,
            &reason,
            home,
            None,
            Some(Question {
                capability,
                approves: Some(Approves::NewTeammate(Box::new(persona))),
            }),
        )?;
        Ok(format!(
            "{asked} {name} does not exist until the person says yes, so do not say it does. On yes, {name} is on the team and message_teammate reaches it by that name; on no, or if the card expires, no teammate is made."
        ))
    }

    /// Called under the card-answer lock, so a second answer or a withdrawal
    /// cannot race the write. The teammate written is the one the card showed,
    /// stamped now; the lead must still lead and its session still be live.
    pub(super) fn settle_new_teammate(
        &self,
        persona_id: &str,
        action_id: &str,
        status: HumanActionStatus,
        expires_at: Option<i64>,
    ) -> Result<(), String> {
        // A no makes nothing, so it needs no live authority to be given.
        if status != HumanActionStatus::Done {
            return Ok(());
        }
        let questions = lock(&self.voice_questions);
        let question = questions
            .get(action_id)
            .ok_or("This teammate approval no longer has a live session. Ask again.")?;
        let Some(Approves::NewTeammate(persona)) = &question.approves else {
            return Err("This card cannot add a teammate.".into());
        };
        question.capability.check()?;
        if expires_at.is_none_or(|expires| expires <= now_ms()) {
            return Err("This teammate approval has expired. Ask again.".into());
        }
        self.check_lead(persona_id)?;
        let stamped = now_ms();
        let persona = Persona {
            created_at: stamped,
            updated_at: stamped,
            ..(**persona).clone()
        };
        add_persona(&self.log, &persona)
    }

    /// The agent a lead named, by id or by the name `list_teammates` shows.
    async fn agent_id(&self, agent: &str) -> Result<String, String> {
        if agent == HOTLINE_BACKEND_ID || agent.eq_ignore_ascii_case("Hotline Agent") {
            return Ok(HOTLINE_BACKEND_ID.to_string());
        }
        let backends = acp::registry::backends(self.log.root()).await;
        let Some(backend) = backends
            .iter()
            .find(|backend| backend.id == agent || backend.name.eq_ignore_ascii_case(agent))
        else {
            let ready: Vec<&str> = backends
                .iter()
                .filter(|backend| backend.unavailable.is_none())
                .map(|backend| backend.name.as_str())
                .collect();
            return Err(format!(
                "There is no agent called {agent} on this desk. Hotline Agent{}{} can start here.",
                if ready.is_empty() { "" } else { ", " },
                ready.join(", ")
            ));
        };
        match &backend.unavailable {
            Some(missing) => Err(format!("{} is not ready: {missing}", backend.name)),
            None => Ok(backend.id.clone()),
        }
    }
}
