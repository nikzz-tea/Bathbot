mod add_bg;
mod bigger;
mod hint;
mod rankings;
mod start;
mod stop;
mod tags;

pub use add_bg::*;
pub use bigger::*;
pub use hint::*;
pub use rankings::*;
pub use start::*;
pub use stop::*;
pub use tags::*;

use crate::{
    embeds::{BGHelpEmbed, EmbedData},
    util::MessageExt,
    Args, BotResult, Context,
};

use std::sync::Arc;
use twilight::model::channel::{Message, Reaction};

#[command]
#[short_desc("Play the background guessing game")]
#[aliases("bg")]
#[sub_commands(start, bigger, hint, stop, rankings)]
pub async fn backgroundgame(ctx: Arc<Context>, msg: &Message, mut args: Args) -> BotResult<()> {
    match args.next() {
        None | Some("help") => {
            let prefix = ctx.config_first_prefix(msg.guild_id);
            let embed = BGHelpEmbed::new(prefix).build().build();
            msg.build_response(&ctx, |m| m.embed(embed)).await
        }
        _ => {
            let prefix = ctx.config_first_prefix(msg.guild_id);
            let content = format!(
                "That's not a valid subcommand. Check `{}bg` for more help.",
                prefix
            );
            msg.respond(&ctx, content).await
        }
    }
}

enum ReactionWrapper {
    Add(Reaction),
    Remove(Reaction),
}

impl ReactionWrapper {
    fn as_deref(&self) -> &Reaction {
        match self {
            Self::Add(r) | Self::Remove(r) => r,
        }
    }
}
