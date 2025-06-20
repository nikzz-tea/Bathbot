use std::borrow::Cow;

use bathbot_model::{Countries, MedalCount, OsekaiUserEntry};
use bathbot_util::{Authored, constants::GENERAL_ISSUE};
use eyre::{Report, Result};

use super::OsekaiMedalCount;
use crate::{
    Context,
    active::{ActiveMessages, impls::MedalCountPagination},
    util::{InteractionCommandExt, interaction::InteractionCommand},
};

pub(super) async fn medal_count(
    mut command: InteractionCommand,
    args: OsekaiMedalCount,
) -> Result<()> {
    let country_code = match args.country {
        Some(country) => {
            if country.len() == 2 {
                Some(Cow::Owned(country))
            } else if let Some(code) = Countries::name(&country).to_code() {
                Some(code.into())
            } else {
                let content =
                    format!("Looks like `{country}` is neither a country name nor a country code");

                command.error(content).await?;

                return Ok(());
            }
        }
        None => None,
    };

    let owner = command.user_id()?;
    let ranking_fut = Context::redis().osekai_ranking::<MedalCount>();
    let config_fut = Context::user_config().osu_name(owner);

    let (osekai_res, name_res) = tokio::join!(ranking_fut, config_fut);

    let mut ranking = match osekai_res {
        Ok(ranking) => ranking.try_deserialize::<Vec<OsekaiUserEntry>>().unwrap(),
        Err(err) => {
            let _ = command.error(GENERAL_ISSUE).await;

            return Err(Report::new(err).wrap_err("Failed to get cached medal count ranking"));
        }
    };

    let author_name = match name_res {
        Ok(name_opt) => name_opt,
        Err(err) => {
            warn!(?err, "Failed to get username");

            None
        }
    };

    if let Some(code) = country_code {
        let code = code.to_ascii_uppercase();

        ranking.retain(|entry| entry.country_code == code);
    }

    let author_idx = author_name
        .as_deref()
        .and_then(|name| ranking.iter().position(|e| e.username.as_str() == name));

    let pagination = MedalCountPagination::builder()
        .ranking(ranking.into_boxed_slice())
        .author_idx(author_idx)
        .msg_owner(owner)
        .build();

    ActiveMessages::builder(pagination)
        .start_by_update(true)
        .begin(&mut command)
        .await
}
