use std::borrow::Cow;

use bathbot_macros::{HasName, SlashCommand};
use eyre::Result;
use twilight_interactions::command::{CommandModel, CreateCommand};
use twilight_model::id::{Id, marker::UserMarker};

use crate::{
    commands::{DISCORD_OPTION_DESC, DISCORD_OPTION_HELP},
    util::{InteractionCommandExt, interaction::InteractionCommand},
};

mod user;

#[derive(CommandModel, CreateCommand, SlashCommand)]
#[command(name = "dailychallenge", desc = "Daily challenge statistics")]
pub enum DailyChallenge<'a> {
    #[command(name = "user")]
    User(DailyChallengeUser<'a>),
}

const DC_USER_DESC: &str = "Daily challenge statistics of a user";

#[derive(CommandModel, CreateCommand, HasName)]
#[command(name = "user", desc = DC_USER_DESC)]
pub struct DailyChallengeUser<'a> {
    #[command(desc = "Specify a username")]
    name: Option<Cow<'a, str>>,
    #[command(desc = DISCORD_OPTION_DESC, help = DISCORD_OPTION_HELP)]
    discord: Option<Id<UserMarker>>,
}

async fn slash_dailychallenge(mut command: InteractionCommand) -> Result<()> {
    match DailyChallenge::from_interaction(command.input_data())? {
        DailyChallenge::User(user) => user::user((&mut command).into(), user).await,
    }
}
