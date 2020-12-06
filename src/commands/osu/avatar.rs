use crate::{
    arguments::{Args, NameArgs},
    embeds::{AvatarEmbed, EmbedData},
    util::{constants::OSU_API_ISSUE, MessageExt},
    BotResult, Context,
};

use rosu::model::GameMode;
use std::sync::Arc;
use twilight_model::channel::Message;

#[command]
#[short_desc("Display someone's osu! profile picture")]
#[aliases("pfp")]
#[usage("[username]")]
#[example("Badewanne3")]
async fn avatar(ctx: Arc<Context>, msg: &Message, args: Args) -> BotResult<()> {
    let args = NameArgs::new(&ctx, args);
    let name = match args.name.or_else(|| ctx.get_link(msg.author.id.0)) {
        Some(name) => name,
        None => return super::require_link(&ctx, msg).await,
    };
    let user = match ctx.osu().user(name.as_str()).mode(GameMode::STD).await {
        Ok(user) => match user {
            Some(user) => user,
            None => {
                let content = format!("User `{}` was not found", name);
                return msg.error(&ctx, content).await;
            }
        },
        Err(why) => {
            let _ = msg.error(&ctx, OSU_API_ISSUE).await;
            return Err(why.into());
        }
    };
    let embed = AvatarEmbed::new(user).build().build()?;
    msg.build_response(&ctx, |m| m.embed(embed)).await?;
    Ok(())
}
