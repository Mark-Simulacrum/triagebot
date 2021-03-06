//! Permit assignment of any user to issues, without requiring "write" access to the repository.
//!
//! We need to fake-assign ourselves and add a 'claimed by' section to the top-level comment.
//!
//! Such assigned issues should also be placed in a queue to ensure that the user remains
//! active; the assigned user will be asked for a status report every 2 weeks (XXX: timing).
//!
//! If we're intending to ask for a status report but no comments from the assigned user have
//! been given for the past 2 weeks, the bot will de-assign the user. They can once more claim
//! the issue if necessary.
//!
//! Assign users with `@rustbot assign @gh-user` or `@rustbot claim` (self-claim).

use crate::{
    config::AssignConfig,
    github::{self, Event},
    handlers::{Context, Handler},
    interactions::EditIssueBody,
};
use failure::{Error, ResultExt};
use parser::command::assign::AssignCommand;
use parser::command::{Command, Input};

pub(super) struct AssignmentHandler;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AssignData {
    user: Option<String>,
}

impl Handler for AssignmentHandler {
    type Input = AssignCommand;
    type Config = AssignConfig;

    fn parse_input(&self, ctx: &Context, event: &Event) -> Result<Option<Self::Input>, Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(None);
        };

        let mut input = Input::new(&event.comment.body, &ctx.username);
        match input.parse_command() {
            Command::Assign(Ok(command)) => Ok(Some(command)),
            Command::Assign(Err(err)) => {
                failure::bail!(
                    "Parsing assign command in [comment]({}) failed: {}",
                    event.comment.html_url,
                    err
                );
            }
            _ => Ok(None),
        }
    }

    fn handle_input(
        &self,
        ctx: &Context,
        _config: &AssignConfig,
        event: &Event,
        cmd: AssignCommand,
    ) -> Result<(), Error> {
        #[allow(irrefutable_let_patterns)]
        let event = if let Event::IssueComment(e) = event {
            e
        } else {
            // not interested in other events
            return Ok(());
        };

        let is_team_member =
            if let Err(_) | Ok(false) = event.comment.user.is_team_member(&ctx.github) {
                false
            } else {
                true
            };

        let e = EditIssueBody::new(&event.issue, "ASSIGN");

        let to_assign = match cmd {
            AssignCommand::Own => event.comment.user.login.clone(),
            AssignCommand::User { username } => {
                if is_team_member {
                    if username != event.comment.user.login {
                        failure::bail!("Only Rust team members can assign other users");
                    }
                }
                username.clone()
            }
            AssignCommand::Release => {
                let current = if let Some(AssignData { user: Some(user) }) = e.current_data() {
                    user
                } else {
                    failure::bail!("Cannot release unassigned issue");
                };
                if current == event.comment.user.login || is_team_member {
                    event.issue.remove_assignees(&ctx.github)?;
                    e.apply(&ctx.github, String::new(), AssignData { user: None })?;
                    return Ok(());
                } else {
                    failure::bail!("Cannot release another user's assignment");
                }
            }
        };
        let data = AssignData {
            user: Some(to_assign.clone()),
        };

        e.apply(&ctx.github, String::new(), &data)?;

        match event.issue.set_assignee(&ctx.github, &to_assign) {
            Ok(()) => return Ok(()), // we are done
            Err(github::AssignmentError::InvalidAssignee) => {
                event
                    .issue
                    .set_assignee(&ctx.github, &ctx.username)
                    .context("self-assignment failed")?;
                e.apply(
                    &ctx.github,
                    format!(
                        "This issue has been assigned to @{} via [this comment]({}).",
                        to_assign, event.comment.html_url
                    ),
                    &data,
                )?;
            }
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }
}
