use std::{fmt::Write, sync::Arc};

use prometheus::core::Collector;
use twilight_model::{
    application::{
        callback::{Autocomplete, CallbackData, InteractionResponse},
        command::CommandOptionChoice,
        component::{
            button::ButtonStyle, select_menu::SelectMenuOption, ActionRow, Button, Component,
            SelectMenu,
        },
        interaction::{
            application_command::CommandOptionValue, ApplicationCommand,
            MessageComponentInteraction,
        },
    },
    channel::embed::EmbedField,
};

use crate::{
    commands::{MyCommand, MyCommandOption, MyCommandOptionKind, SLASH_COMMANDS},
    core::Context,
    embeds::{EmbedBuilder, Footer},
    error::{Error, InvalidHelpState},
    util::{
        constants::{common_literals::HELP, BATHBOT_GITHUB, BATHBOT_WORKSHOP, INVITE_LINK},
        datetime::how_long_ago_dynamic,
        levenshtein_distance,
        numbers::with_comma_int,
        CowUtils, MessageBuilder, MessageExt,
    },
    BotResult,
};

use super::failed_message_;

type PartResult = Result<(Parts, bool), InvalidHelpState>;

struct Parts {
    name: &'static str,
    help: &'static str,
    root: bool,
    options: Vec<MyCommandOption>,
}

impl From<MyCommand> for Parts {
    fn from(command: MyCommand) -> Self {
        Self {
            name: command.name,
            help: command.help.unwrap_or(command.description),
            root: true,
            options: command.options,
        }
    }
}

impl From<MyCommandOption> for Parts {
    fn from(option: MyCommandOption) -> Self {
        let options = match option.kind {
            MyCommandOptionKind::SubCommand { options }
            | MyCommandOptionKind::SubCommandGroup { options } => options,
            MyCommandOptionKind::String { .. }
            | MyCommandOptionKind::Integer { .. }
            | MyCommandOptionKind::Number { .. }
            | MyCommandOptionKind::Boolean { .. }
            | MyCommandOptionKind::User { .. }
            | MyCommandOptionKind::Channel { .. }
            | MyCommandOptionKind::Role { .. }
            | MyCommandOptionKind::Mentionable { .. } => Vec::new(),
        };

        Self {
            name: option.name,
            help: option.help.unwrap_or(option.description),
            root: false,
            options,
        }
    }
}

impl From<EitherCommand> for Parts {
    fn from(either: EitherCommand) -> Self {
        match either {
            EitherCommand::Base(command) => command.into(),
            EitherCommand::Option(option) => option.into(),
        }
    }
}

impl From<CommandIter> for Parts {
    fn from(iter: CommandIter) -> Self {
        match iter.next {
            Some(option) => option.into(),
            None => iter.curr.into(),
        }
    }
}

enum EitherCommand {
    Base(MyCommand),
    Option(MyCommandOption),
}

struct CommandIter {
    curr: EitherCommand,
    next: Option<MyCommandOption>,
}

impl From<MyCommand> for CommandIter {
    fn from(command: MyCommand) -> Self {
        Self {
            curr: EitherCommand::Base(command),
            next: None,
        }
    }
}

impl CommandIter {
    fn next(&mut self, name: &str) -> bool {
        let options = match &mut self.next {
            Some(option) => match &mut option.kind {
                MyCommandOptionKind::SubCommand { options }
                | MyCommandOptionKind::SubCommandGroup { options } => options,
                _ => return true,
            },
            None => match &mut self.curr {
                EitherCommand::Base(command) => &mut command.options,
                EitherCommand::Option(option) => match &mut option.kind {
                    MyCommandOptionKind::SubCommand { options }
                    | MyCommandOptionKind::SubCommandGroup { options } => options,
                    _ => return true,
                },
            },
        };

        let next = match options.drain(..).find(|option| option.name == name) {
            Some(option) => option,
            None => return true,
        };

        if let Some(curr) = self.next.replace(next) {
            self.curr = EitherCommand::Option(curr);
        }

        false
    }
}

const AUTHORITY_STATUS: &str = "Requires authority status (check the /authorities command)";

fn continue_subcommand(title: &mut String, name: &str) -> PartResult {
    let mut names = title.split(' ');
    let base = names.next().ok_or(InvalidHelpState::MissingTitle)?;

    let command = SLASH_COMMANDS
        .command(base)
        .ok_or(InvalidHelpState::UnknownCommand)?;

    let authority = command.authority;
    let mut iter = CommandIter::from(command);

    for name in names {
        if iter.next(name) {
            return Err(InvalidHelpState::UnknownCommand);
        }
    }

    if iter.next(name) {
        return Err(InvalidHelpState::UnknownCommand);
    }

    let command = Parts::from(iter);
    let _ = write!(title, " {}", command.name);

    Ok((command, authority))
}

fn backtrack_subcommand(title: &mut String) -> PartResult {
    let index = title.chars().filter(char::is_ascii_whitespace).count();
    let mut names = title.split(' ').take(index);
    let base = names.next().ok_or(InvalidHelpState::MissingTitle)?;

    let command = SLASH_COMMANDS
        .command(base)
        .ok_or(InvalidHelpState::UnknownCommand)?;

    let authority = command.authority;
    let mut iter = CommandIter::from(command);

    for name in names {
        if iter.next(name) {
            return Err(InvalidHelpState::UnknownCommand);
        }
    }

    if let Some(pos) = title.rfind(' ') {
        title.truncate(pos);
    }

    Ok((iter.into(), authority))
}

pub async fn handle_menu_select(
    ctx: &Context,
    mut component: MessageComponentInteraction,
) -> BotResult<()> {
    // Parse given component
    let mut title = component
        .message
        .embeds
        .pop()
        .ok_or(InvalidHelpState::MissingEmbed)?
        .title
        .ok_or(InvalidHelpState::MissingTitle)?;

    // If value is None, back button was pressed; otherwise subcommand was picked
    let (command, authority) = match component.data.values.pop() {
        Some(name) => continue_subcommand(&mut title, &name)?,
        None => backtrack_subcommand(&mut title)?,
    };

    // Prepare embed and components
    let mut embed_builder = EmbedBuilder::new()
        .title(title)
        .description(command.help)
        .fields(option_fields(&command.options));

    if authority {
        embed_builder = embed_builder.footer(Footer::new(AUTHORITY_STATUS));
    }

    let mut components = parse_select_menu(&command.options);
    let menu_content = components.get_or_insert_with(|| Vec::with_capacity(1));

    let button_row = ActionRow {
        components: vec![back_button(command.root)],
    };

    menu_content.push(Component::ActionRow(button_row));

    let response = InteractionResponse::UpdateMessage(CallbackData {
        allowed_mentions: None,
        components,
        content: None,
        embeds: Some(vec![embed_builder.build()]),
        flags: None,
        tts: None,
    });

    ctx.interaction()
        .interaction_callback(component.id, &component.token, &response)
        .exec()
        .await?;

    Ok(())
}

fn back_button(disabled: bool) -> Component {
    let button = Button {
        custom_id: Some("help_back".to_owned()),
        disabled,
        emoji: None,
        label: Some("Back".to_owned()),
        style: ButtonStyle::Danger,
        url: None,
    };

    Component::Button(button)
}

fn option_fields(children: &[MyCommandOption]) -> Vec<EmbedField> {
    children
        .iter()
        .filter_map(|child| match &child.kind {
            MyCommandOptionKind::SubCommand { .. }
            | MyCommandOptionKind::SubCommandGroup { .. } => None,
            MyCommandOptionKind::String { required, .. }
            | MyCommandOptionKind::Integer { required, .. }
            | MyCommandOptionKind::Number { required, .. }
            | MyCommandOptionKind::Boolean { required }
            | MyCommandOptionKind::User { required }
            | MyCommandOptionKind::Channel { required }
            | MyCommandOptionKind::Role { required }
            | MyCommandOptionKind::Mentionable { required } => {
                let mut name = child.name.to_owned();

                if *required {
                    name.push_str(" (required)");
                }

                let help = child.help.unwrap_or(child.description);

                let field = EmbedField {
                    inline: help.len() <= 40,
                    name,
                    value: help.to_owned(),
                };

                Some(field)
            }
        })
        .collect()
}

fn parse_select_menu(options: &[MyCommandOption]) -> Option<Vec<Component>> {
    if options.is_empty() {
        return None;
    }

    let options: Vec<_> = options
        .iter()
        .filter(|option| {
            matches!(
                option.kind,
                MyCommandOptionKind::SubCommand { .. }
                    | MyCommandOptionKind::SubCommandGroup { .. }
            )
        })
        .map(|option| SelectMenuOption {
            default: false,
            description: Some(option.description.to_owned()),
            emoji: None,
            label: option.name.to_owned(),
            value: option.name.to_owned(),
        })
        .collect();

    if options.is_empty() {
        return None;
    }

    let select_menu = SelectMenu {
        custom_id: "help_menu".to_owned(),
        disabled: false,
        max_values: None,
        min_values: None,
        options,
        placeholder: Some("Select a subcommand".to_owned()),
    };

    let row = ActionRow {
        components: vec![Component::SelectMenu(select_menu)],
    };

    Some(vec![Component::ActionRow(row)])
}

async fn help_slash_command(
    ctx: &Context,
    command: ApplicationCommand,
    cmd: MyCommand,
) -> BotResult<()> {
    let MyCommand {
        name,
        description,
        help,
        authority,
        options,
    } = cmd;

    let description = help.unwrap_or(description);

    if name == "owner" {
        let description =
            "This command can only be used by the owner of the bot.\nQuit snooping around :^)";

        let embed_builder = EmbedBuilder::new().title(name).description(description);
        let builder = MessageBuilder::new().embed(embed_builder);
        command.create_message(ctx, builder).await?;

        return Ok(());
    }

    let mut embed_builder = EmbedBuilder::new()
        .title(name)
        .description(description)
        .fields(option_fields(&options));

    if authority {
        let footer = Footer::new(AUTHORITY_STATUS);

        embed_builder = embed_builder.footer(footer);
    }

    let menu = parse_select_menu(&options);

    let builder = MessageBuilder::new()
        .embed(embed_builder)
        .components(menu.as_deref().unwrap_or_default());

    command.create_message(ctx, builder).await?;

    Ok(())
}

pub async fn handle_autocomplete(ctx: Arc<Context>, command: ApplicationCommand) -> BotResult<()> {
    let mut cmd_name = None;
    let mut focus = None;

    if let Some(option) = command.data.options.first() {
        let option = (option.name == "command").then(|| match &option.value {
            CommandOptionValue::String(value) => Some((value, option.focused)),
            _ => None,
        });

        match option.flatten() {
            Some((value, focus_)) => {
                cmd_name = Some(value);
                focus = Some(focus_);
            }
            None => return Err(Error::InvalidCommandOptions),
        }
    }

    let name = cmd_name.map(|name| name.cow_to_ascii_lowercase());

    let choices = match (name, focus) {
        (Some(name), Some(true)) => {
            let arg = name.trim();

            match (arg, SLASH_COMMANDS.descendants(arg)) {
                ("", _) | (_, None) => Vec::new(),
                (_, Some(cmds)) => cmds
                    .map(|cmd| CommandOptionChoice::String {
                        name: cmd.to_owned(),
                        value: cmd.to_owned(),
                    })
                    .collect(),
            }
        }
        _ => Vec::new(),
    };

    let res = InteractionResponse::Autocomplete(Autocomplete { choices });

    ctx.interaction()
        .interaction_callback(command.id, &command.token, &res)
        .exec()
        .await?;

    Ok(())
}

pub async fn slash_help(ctx: Arc<Context>, command: ApplicationCommand) -> BotResult<()> {
    let mut cmd_name = None;

    if let Some(option) = command.data.options.first() {
        let option = (option.name == "command").then(|| match &option.value {
            CommandOptionValue::String(value) => Some(value),
            _ => None,
        });

        match option.flatten() {
            Some(value) => cmd_name = Some(value),
            None => return Err(Error::InvalidCommandOptions),
        }
    }

    let name = cmd_name.map(|name| name.cow_to_ascii_lowercase());

    match name {
        Some(name) => {
            let arg = name.as_ref();

            match SLASH_COMMANDS.command(arg) {
                Some(cmd) => help_slash_command(&ctx, command, cmd).await,
                None => {
                    let dists = SLASH_COMMANDS
                        .names()
                        .map(|name| (levenshtein_distance(arg, name).0, name))
                        .filter(|(dist, _)| *dist < 5)
                        .collect();

                    failed_message_(&ctx, command.into(), dists).await
                }
            }
        }
        None => basic_help(&ctx, command).await,
    }
}

async fn basic_help(ctx: &Context, command: ApplicationCommand) -> BotResult<()> {
    let id = ctx
        .cache
        .current_user()
        .expect("missing CurrentUser in cache")
        .id;
    let mention = format!("<@{id}>");

    let description = format!(
        "{mention} is a discord bot written by [Badewanne3](https://osu.ppy.sh/u/2211396) all around osu!"
    );

    let join_server = EmbedField {
        inline: false,
        name: "Got a question, suggestion, bug, or are interested in the development?".to_owned(),
        value: format!("Feel free to join the [discord server]({BATHBOT_WORKSHOP})"),
    };

    let command_help = EmbedField {
        inline: false,
        name: "Want to learn more about a command?".to_owned(),
        value: "Try specifying the command name on the `help` command: `/help command:_`"
            .to_owned(),
    };

    let invite = EmbedField {
        inline: false,
        name: "Want to invite the bot to your server?".to_owned(),
        value: format!("Try using this [**invite link**]({INVITE_LINK})"),
    };

    let boot_time = ctx.stats.start_time;
    let mut fields = vec![join_server, command_help, invite];

    let servers = EmbedField {
        inline: true,
        name: "Servers".to_owned(),
        value: with_comma_int(ctx.cache.stats().guilds()).to_string(),
    };

    fields.push(servers);

    let boot_up = EmbedField {
        inline: true,
        name: "Boot-up".to_owned(),
        value: how_long_ago_dynamic(&boot_time).to_string(),
    };

    let github = EmbedField {
        inline: false,
        name: "Interested in the code?".to_owned(),
        value: format!("The source code can be found over at [github]({BATHBOT_GITHUB})"),
    };

    let commands_used: usize = ctx.stats.command_counts.message_commands.collect()[0]
        .get_metric()
        .iter()
        .map(|metrics| metrics.get_counter().get_value() as usize)
        .sum();

    let osu_requests: usize = ctx.stats.osu_metrics.rosu.collect()[0]
        .get_metric()
        .iter()
        .map(|metric| metric.get_counter().get_value() as usize)
        .sum();

    let commands_used = EmbedField {
        inline: true,
        name: "Commands used".to_owned(),
        value: with_comma_int(commands_used).to_string(),
    };

    let osu_requests = EmbedField {
        inline: true,
        name: "osu!api requests".to_owned(),
        value: with_comma_int(osu_requests).to_string(),
    };

    fields.push(boot_up);
    fields.push(github);
    fields.push(commands_used);
    fields.push(osu_requests);

    let builder = EmbedBuilder::new()
        .description(description)
        .fields(fields)
        .build()
        .into();

    command.create_message(ctx, builder).await?;

    Ok(())
}

pub fn define_help() -> MyCommand {
    let option_help = "Specify a command **base** name.\n\
        Once the help for that command is displayed, you can use the menu to navigate \
        to specific subcommands you want to know more about.";

    let command = MyCommandOption::builder("command", "Specify a command base name")
        .help(option_help)
        .autocomplete()
        .string(Vec::new(), false);

    let description = "Display general help or help for a specific command";
    let help = "If no command name is specified, it will show general help for the bot.\n\
        Otherwise it'll show a help menu for the specific command.";

    MyCommand::new(HELP, description)
        .help(help)
        .options(vec![command])
}
