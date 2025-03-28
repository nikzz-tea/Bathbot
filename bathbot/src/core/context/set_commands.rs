use eyre::{Result, WrapErr};
use twilight_model::{
    application::{command::Command, interaction::InteractionContextType},
    oauth::ApplicationIntegrationType,
};

use super::Context;
use crate::core::BotConfig;

impl Context {
    #[cold]
    pub async fn set_global_commands(mut cmds: Vec<Command>) -> Result<Vec<Command>> {
        let integrations = vec![
            ApplicationIntegrationType::GuildInstall,
            ApplicationIntegrationType::UserInstall,
        ];

        for cmd in cmds.iter_mut() {
            if cmd.integration_types.is_none() {
                cmd.integration_types = Some(integrations.clone());
            } else {
                warn!(command = cmd.name, "Command integrations already set");
            }

            #[allow(deprecated)]
            let contexts = if cmd.dm_permission == Some(false) {
                vec![InteractionContextType::Guild]
            } else {
                vec![
                    InteractionContextType::Guild,
                    InteractionContextType::BotDm,
                    InteractionContextType::PrivateChannel,
                ]
            };

            if cmd.contexts.is_none() {
                cmd.contexts = Some(contexts);
            } else {
                warn!(command = cmd.name, "Command contexts already set");
            }
        }

        Context::interaction()
            .set_global_commands(&cmds)
            .await
            .wrap_err("Failed to set commands")?
            .models()
            .await
            .wrap_err("Failed to deserialize commands")
    }

    #[cold]
    pub async fn set_guild_commands(cmds: Vec<Command>) -> Result<Vec<Command>> {
        Context::interaction()
            .set_guild_commands(BotConfig::get().dev_guild, &cmds)
            .await
            .wrap_err("Failed to set commands")?
            .models()
            .await
            .wrap_err("Failed to deserialize commands")
    }
}
