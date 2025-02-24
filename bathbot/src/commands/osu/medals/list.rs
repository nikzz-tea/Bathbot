use std::{
    cmp::{Ordering, Reverse},
    collections::HashMap,
};

use bathbot_model::{OsekaiMedal, Rarity};
use bathbot_util::{IntHasher, constants::GENERAL_ISSUE};
use eyre::{Report, Result};
use rkyv::{
    rancor::{Panic, ResultExt},
    vec::ArchivedVec,
};
use rosu_v2::{model::GameMode, prelude::OsuError, request::UserId};
use time::OffsetDateTime;

use super::{MedalList, MedalListOrder};
use crate::{
    Context,
    active::{ActiveMessages, impls::MedalsListPagination},
    commands::osu::{require_link, user_not_found},
    core::commands::CommandOrigin,
    manager::redis::osu::{UserArgs, UserArgsError},
};

pub(super) async fn list(orig: CommandOrigin<'_>, args: MedalList<'_>) -> Result<()> {
    let owner = orig.user_id()?;

    let user_id = match user_id!(orig, args) {
        Some(user_id) => user_id,
        None => match Context::user_config().osu_id(owner).await {
            Ok(Some(user_id)) => UserId::Id(user_id),
            Ok(None) => return require_link(&orig).await,
            Err(err) => {
                let _ = orig.error(GENERAL_ISSUE).await;

                return Err(err);
            }
        },
    };

    let MedalList {
        sort,
        group,
        reverse,
        ..
    } = args;

    let user_args = UserArgs::rosu_id(&user_id, GameMode::Osu).await;
    let user_fut = Context::redis().osu_user(user_args);
    let medals_fut = Context::redis().medals();
    let ranking_fut = Context::redis().osekai_ranking::<Rarity>();

    let (user, osekai_medals, rarities) = match tokio::join!(user_fut, medals_fut, ranking_fut) {
        (Ok(user), Ok(mut medals), Ok(rarities)) => {
            medals.mutate(|medals| {
                let medals = ArchivedVec::as_slice_seal(medals);
                // SAFETY: We just sort; no nefarious moving or writing going on
                let medals = unsafe { medals.unseal_unchecked() };

                medals.sort_unstable_by_key(|medal| medal.medal_id);
            });

            (user, medals, rarities)
        }
        (Err(UserArgsError::Osu(OsuError::NotFound)), ..) => {
            let content = user_not_found(user_id).await;

            return orig.error(content).await;
        }
        (Err(err), ..) => {
            let _ = orig.error(GENERAL_ISSUE).await;
            let report = Report::new(err).wrap_err("Failed to get user");

            return Err(report);
        }
        (_, Err(err), _) | (.., Err(err)) => {
            let _ = orig.error(GENERAL_ISSUE).await;

            return Err(Report::new(err).wrap_err("Failed to get cached rarity ranking"));
        }
    };

    let rarities: HashMap<_, _, IntHasher> = rarities
        .iter()
        .map(|entry| {
            (
                entry.medal_id.to_native(),
                entry.possession_percent.to_native(),
            )
        })
        .collect();

    let acquired = (user.medals.len(), osekai_medals.len());

    let medals_iter = user.medals.iter().filter_map(|m| {
        match osekai_medals
            .iter()
            .position(|m_| m_.medal_id == m.medal_id)
        {
            Some(idx) => {
                let achieved = m.achieved_at.try_deserialize::<Panic>().always_ok();

                let entry = MedalEntryList {
                    medal: rkyv::api::deserialize_using::<_, _, Panic>(
                        &osekai_medals[idx],
                        &mut (),
                    )
                    .always_ok(),
                    achieved,
                    rarity: rarities
                        .get(&m.medal_id.to_native())
                        .copied()
                        .unwrap_or(100.0),
                };

                Some(entry)
            }
            None => {
                warn!("Missing medal id {}", m.medal_id);

                None
            }
        }
    });

    let mut medals = Vec::with_capacity(acquired.0);
    medals.extend(medals_iter);

    if let Some(group) = group {
        medals.retain(|entry| entry.medal.grouping == group);
    }

    let order_str = match sort.unwrap_or_default() {
        MedalListOrder::Alphabet => {
            medals.sort_unstable_by(|a, b| a.medal.name.cmp(&b.medal.name));

            "alphabet"
        }
        MedalListOrder::Date => {
            medals.sort_unstable_by_key(|entry| Reverse(entry.achieved));

            "date"
        }
        MedalListOrder::MedalId => {
            medals.sort_unstable_by_key(|entry| entry.medal.medal_id);

            "medal id"
        }
        MedalListOrder::Rarity => {
            medals.sort_unstable_by(|a, b| {
                a.rarity.partial_cmp(&b.rarity).unwrap_or(Ordering::Equal)
            });

            "rarity"
        }
    };

    let reverse_str = if reverse == Some(true) {
        medals.reverse();

        "reversed "
    } else {
        ""
    };

    let name = user.username.as_str();

    let content = match group {
        None => format!("All medals of `{name}` sorted by {reverse_str}{order_str}:",),
        Some(group) => {
            format!("All `{group}` medals of `{name}` sorted by {reverse_str}{order_str}:",)
        }
    };

    let pagination = MedalsListPagination::builder()
        .user(user)
        .acquired(acquired)
        .medals(medals.into_boxed_slice())
        .content(content.into_boxed_str())
        .msg_owner(owner)
        .build();

    ActiveMessages::builder(pagination)
        .start_by_update(true)
        .begin(orig)
        .await
}

pub struct MedalEntryList {
    pub medal: OsekaiMedal,
    pub achieved: OffsetDateTime,
    pub rarity: f32,
}
