use std::borrow::Cow;

use bathbot_macros::SlashCommand;
use bathbot_model::{Countries, RankingKind, UserModeStatsColumn, UserStatsColumn, UserStatsKind};
use bathbot_util::{Authored, constants::GENERAL_ISSUE};
use eyre::Result;
use rosu_v2::prelude::GameMode;
use twilight_interactions::command::{CommandModel, CreateCommand};

use crate::{
    Context,
    active::{ActiveMessages, impls::RankingPagination},
    core::commands::interaction::InteractionCommands,
    util::{InteractionCommandExt, interaction::InteractionCommand},
};

#[derive(CommandModel, CreateCommand, SlashCommand)]
#[command(
    name = "serverleaderboard",
    dm_permission = false,
    desc = "Various osu! leaderboards for linked server members",
    help = "Various osu! leaderboards for linked server members.\n\
    Whenever any command is used that requests an osu! user, the retrieved user will be cached.\n\
    The leaderboards will contain all members of this server that are linked to an osu! username \
    which was cached through some command beforehand.\n\
    Since only the cached data is used, no values are guaranteed to be up-to-date. \
    They're just snapshots from the last time the user was retrieved through a command.\n\n\
    There are three reasons why a user might be missing from the leaderboard:\n\
    - They are not linked through the `/link` command\n\
    - Their osu! user stats have not been cached yet. \
    Try using any command that retrieves the user, e.g. `/profile`, in order to cache them.\n\
    - Members of this server are not stored as such. Maybe let bade know :eyes:"
)]
#[flags(ONLY_GUILDS)]
pub enum ServerLeaderboard {
    #[command(name = "all_modes")]
    AllModes(ServerLeaderboardAllModes),
    #[command(name = "osu")]
    Osu(ServerLeaderboardOsu),
    #[command(name = "taiko")]
    Taiko(ServerLeaderboardTaiko),
    #[command(name = "ctb")]
    Catch(ServerLeaderboardCatch),
    #[command(name = "mania")]
    Mania(ServerLeaderboardMania),
}

impl ServerLeaderboard {
    fn country(&self) -> Option<&str> {
        match self {
            Self::AllModes(args) => args.country.as_deref(),
            Self::Osu(args) => args.country.as_deref(),
            Self::Taiko(args) => args.country.as_deref(),
            Self::Catch(args) => args.country.as_deref(),
            Self::Mania(args) => args.country.as_deref(),
        }
    }
}

#[derive(CommandModel, CreateCommand)]
#[command(
    name = "all_modes",
    desc = "Various leaderboards across all modes for linked server members"
)]
pub struct ServerLeaderboardAllModes {
    #[command(
        desc = "Specify what kind of leaderboard to show",
        help = "Specify what kind of leaderboard to show.\
        Notably:\n\
        - `Comments`: Considers comments on things like osu! articles or mapsets\n\
        - `Played maps`: Only maps with leaderboards count i.e. ranked, loved, or approved maps"
    )]
    kind: UserStatsColumn,
    #[command(desc = "Specify a country (code)")]
    country: Option<String>,
}

#[derive(CommandModel, CreateCommand)]
#[command(
    name = "osu",
    desc = "Various osu!standard leaderboards for linked server members"
)]
pub struct ServerLeaderboardOsu {
    #[command(desc = "Specify what kind of leaderboard to show")]
    kind: UserModeStatsColumn,
    #[command(desc = "Specify a country (code)")]
    country: Option<String>,
}

#[derive(CommandModel, CreateCommand)]
#[command(
    name = "taiko",
    desc = "Various osu!taiko leaderboards for linked server members"
)]
pub struct ServerLeaderboardTaiko {
    #[command(desc = "Specify what kind of leaderboard to show")]
    kind: UserModeStatsColumn,
    #[command(desc = "Specify a country (code)")]
    country: Option<String>,
}

#[derive(CommandModel, CreateCommand)]
#[command(
    name = "ctb",
    desc = "Various osu!ctb leaderboards for linked server members"
)]
pub struct ServerLeaderboardCatch {
    #[command(desc = "Specify what kind of leaderboard to show")]
    kind: UserModeStatsColumn,
    #[command(desc = "Specify a country (code)")]
    country: Option<String>,
}

#[derive(CommandModel, CreateCommand)]
#[command(
    name = "mania",
    desc = "Various osu!mania leaderboards for linked server members"
)]
pub struct ServerLeaderboardMania {
    #[command(desc = "Specify what kind of leaderboard to show")]
    kind: UserModeStatsColumn,
    #[command(desc = "Specify a country (code)")]
    country: Option<String>,
}

async fn country_code<'a>(
    command: &InteractionCommand,
    country: &'a str,
) -> Result<Option<Cow<'a, str>>> {
    match Countries::name(country).to_code() {
        Some(code) => Ok(Some(code.into())),
        None if country.len() == 2 => Ok(Some(country.to_ascii_uppercase().into())),
        None => {
            let content =
                format!("Looks like `{country}` is neither a country name nor a country code");

            command.error(content).await?;

            Ok(None)
        }
    }
}

async fn slash_serverleaderboard(mut command: InteractionCommand) -> Result<()> {
    let args = ServerLeaderboard::from_interaction(command.input_data())?;

    let owner = command.user_id()?;
    let guild_id = command.guild_id.unwrap(); // command is only processed in guilds
    let cache = Context::cache();

    let members: Vec<_> = match cache.members(guild_id).await {
        Ok(members) => members.into_iter().map(|id| id as i64).collect(),
        Err(err) => {
            let _ = command.error(GENERAL_ISSUE).await;

            return Err(err);
        }
    };

    let guild_icon = cache
        .guild(guild_id)
        .await
        .ok()
        .flatten()
        .and_then(|guild| Some((guild.id.to_native(), *guild.icon.as_ref()?)));

    let author_name_fut = Context::user_config().osu_name(owner);

    let ((author_name_res, entries_res), kind) = match &args {
        ServerLeaderboard::AllModes(args) => {
            let country_code = match args.country.as_deref() {
                Some(country) => match country_code(&command, country).await? {
                    code @ Some(_) => code,
                    None => return Ok(()),
                },
                None => None,
            };

            let entries_fut =
                Context::osu_user().stats(&members, args.kind, country_code.as_deref());

            let kind = RankingKind::UserStats {
                guild_icon,
                kind: UserStatsKind::AllModes { column: args.kind },
            };

            (tokio::join!(author_name_fut, entries_fut), kind)
        }
        ServerLeaderboard::Osu(args) => {
            let country_code = match args.country.as_deref() {
                Some(country) => match country_code(&command, country).await? {
                    code @ Some(_) => code,
                    None => return Ok(()),
                },
                None => None,
            };

            let entries_fut = Context::osu_user().stats_mode(
                &members,
                GameMode::Osu,
                args.kind,
                country_code.as_deref(),
            );

            let kind = RankingKind::UserStats {
                guild_icon,
                kind: UserStatsKind::Mode {
                    mode: GameMode::Osu,
                    column: args.kind,
                },
            };

            (tokio::join!(author_name_fut, entries_fut), kind)
        }
        ServerLeaderboard::Taiko(args) => {
            let country_code = match args.country.as_deref() {
                Some(country) => match country_code(&command, country).await? {
                    code @ Some(_) => code,
                    None => return Ok(()),
                },
                None => None,
            };

            let entries_fut = Context::osu_user().stats_mode(
                &members,
                GameMode::Taiko,
                args.kind,
                country_code.as_deref(),
            );

            let kind = RankingKind::UserStats {
                guild_icon,
                kind: UserStatsKind::Mode {
                    mode: GameMode::Taiko,
                    column: args.kind,
                },
            };

            (tokio::join!(author_name_fut, entries_fut), kind)
        }
        ServerLeaderboard::Catch(args) => {
            let country_code = match args.country.as_deref() {
                Some(country) => match country_code(&command, country).await? {
                    code @ Some(_) => code,
                    None => return Ok(()),
                },
                None => None,
            };

            let entries_fut = Context::osu_user().stats_mode(
                &members,
                GameMode::Catch,
                args.kind,
                country_code.as_deref(),
            );

            let kind = RankingKind::UserStats {
                guild_icon,
                kind: UserStatsKind::Mode {
                    mode: GameMode::Catch,
                    column: args.kind,
                },
            };

            (tokio::join!(author_name_fut, entries_fut), kind)
        }
        ServerLeaderboard::Mania(args) => {
            let country_code = match args.country.as_deref() {
                Some(country) => match country_code(&command, country).await? {
                    code @ Some(_) => code,
                    None => return Ok(()),
                },
                None => None,
            };

            let entries_fut = Context::osu_user().stats_mode(
                &members,
                GameMode::Mania,
                args.kind,
                country_code.as_deref(),
            );

            let kind = RankingKind::UserStats {
                guild_icon,
                kind: UserStatsKind::Mode {
                    mode: GameMode::Mania,
                    column: args.kind,
                },
            };

            (tokio::join!(author_name_fut, entries_fut), kind)
        }
    };

    let entries = match entries_res {
        Ok(entries) => entries,
        Err(err) => {
            let _ = command.error(GENERAL_ISSUE).await;

            return Err(err);
        }
    };

    let author_name = match author_name_res {
        Ok(name_opt) => name_opt,
        Err(err) => {
            warn!("{err:?}");

            None
        }
    };

    if entries.is_empty() {
        let content = if args.country().is_some() {
            "No user data found for members of this server from that country".to_owned()
        } else {
            let link = InteractionCommands::get_command("link").map_or_else(
                || "`/link`".to_owned(),
                |cmd| cmd.mention("link").to_string(),
            );

            let profile = InteractionCommands::get_command("profile").map_or_else(
                || "`/profile`".to_owned(),
                |cmd| cmd.mention("profile").to_string(),
            );

            format!(
                "No user data found for members of this server :(\n\
                There could be three reasons:\n\
                - Members of this server are not linked through the {link} command\n\
                - Their osu! user stats have not been cached yet. \
                Try using any command that retrieves an osu! user, e.g. {profile}, \
                in order to cache them.\n\
                - Members of this server are not stored as such. Maybe let bade know :eyes:"
            )
        };

        command.error(content).await?;

        return Ok(());
    }

    let author_idx = author_name.and_then(|name| entries.name_pos(&name));
    let total = entries.len();

    let pagination = RankingPagination::builder()
        .entries(entries)
        .total(total)
        .author_idx(author_idx)
        .kind(kind)
        .defer(false)
        .msg_owner(owner)
        .build();

    ActiveMessages::builder(pagination)
        .start_by_update(true)
        .begin(&mut command)
        .await
}
