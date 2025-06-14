use std::{borrow::Cow, collections::HashMap};

use bathbot_macros::command;
use bathbot_model::rosu_v2::user::MedalCompactRkyv;
use bathbot_util::{IntHasher, MessageBuilder, constants::GENERAL_ISSUE, matcher};
use eyre::{ContextCompat, Report, Result, WrapErr};
use plotters::prelude::*;
use plotters_skia::SkiaBackend;
use rkyv::{
    rancor::{Panic, ResultExt},
    with::{Map, With},
};
use rosu_v2::{
    model::GameMode,
    prelude::{MedalCompact, OsuError},
    request::UserId,
};
use skia_safe::{EncodedImageFormat, surfaces};
use time::OffsetDateTime;
use twilight_model::guild::Permissions;

use super::MedalStats;
use crate::{
    Context,
    commands::osu::{require_link, user_not_found},
    core::commands::CommandOrigin,
    embeds::{EmbedData, MedalStatsEmbed, StatsMedal},
    manager::redis::osu::{UserArgs, UserArgsError},
    util::Monthly,
};

#[command]
#[desc("Display medal stats for a user")]
#[usage("[username]")]
#[examples("badewanne3", r#""im a fancy lad""#)]
#[alias("ms")]
#[group(AllModes)]
async fn prefix_medalstats(
    msg: &Message,
    mut args: Args<'_>,
    permissions: Option<Permissions>,
) -> Result<()> {
    let args = match args.next() {
        Some(arg) => match matcher::get_mention_user(arg) {
            Some(id) => MedalStats {
                name: None,
                discord: Some(id),
            },
            None => MedalStats {
                name: Some(Cow::Borrowed(arg)),
                discord: None,
            },
        },
        None => MedalStats::default(),
    };

    stats(CommandOrigin::from_msg(msg, permissions), args).await
}

pub(super) async fn stats(orig: CommandOrigin<'_>, args: MedalStats<'_>) -> Result<()> {
    let user_id = match user_id!(orig, args) {
        Some(user_id) => user_id,
        None => match Context::user_config().osu_id(orig.user_id()?).await {
            Ok(Some(user_id)) => UserId::Id(user_id),
            Ok(None) => return require_link(&orig).await,
            Err(err) => {
                let _ = orig.error(GENERAL_ISSUE).await;

                return Err(err);
            }
        },
    };

    let user_args = UserArgs::rosu_id(&user_id, GameMode::Osu).await;
    let user_fut = Context::redis().osu_user(user_args);
    let medals_fut = Context::redis().medals();

    let (user, all_medals) = match tokio::join!(user_fut, medals_fut) {
        (Ok(user), Ok(medals)) => (user, medals),
        (Err(UserArgsError::Osu(OsuError::NotFound)), _) => {
            let content = user_not_found(user_id).await;

            return orig.error(content).await;
        }
        (_, Err(err)) => {
            let _ = orig.error(GENERAL_ISSUE).await;

            return Err(Report::new(err).wrap_err("Failed to get cached medals"));
        }
        (Err(err), _) => {
            let _ = orig.error(GENERAL_ISSUE).await;

            return Err(Report::new(err).wrap_err("Failed to get user"));
        }
    };

    let mut medals = rkyv::api::deserialize_using::<_, _, Panic>(
        With::<_, Map<MedalCompactRkyv>>::cast(&user.medals),
        &mut (),
    )
    .always_ok();

    medals.sort_unstable_by_key(|medal| medal.achieved_at);

    let graph = match graph(&medals, W, H) {
        Ok(bytes_option) => bytes_option,
        Err(err) => {
            warn!(?err, "Failed to create graph");

            None
        }
    };

    let all_medals: HashMap<_, _, IntHasher> = all_medals
        .iter()
        .map(|medal| {
            let medal_id = medal.medal_id;

            let medal = StatsMedal {
                name: medal.name.as_ref().into(),
                group: rkyv::api::deserialize_using::<_, _, Panic>(&medal.grouping, &mut ())
                    .always_ok(),
                rarity: medal.rarity.as_ref().map_or(0.0, |n| n.to_native()),
            };

            (medal_id.to_native(), medal)
        })
        .collect();

    let rarest = medals
        .iter()
        .filter_map(|medal| Some((all_medals.get(&medal.medal_id)?.rarity, medal)))
        .reduce(|rarest, next| if next.0 < rarest.0 { next } else { rarest })
        .map(|(_, medal)| *medal);

    let embed = MedalStatsEmbed::new(&user, &medals, &all_medals, rarest, graph.is_some()).build();
    let mut builder = MessageBuilder::new().embed(embed);

    if let Some(graph) = graph {
        builder = builder.attachment("medal_graph.png", graph);
    }

    orig.create_message(builder).await?;

    Ok(())
}

const W: u32 = 1350;
const H: u32 = 350;

pub fn graph(medals: &[MedalCompact], w: u32, h: u32) -> Result<Option<Vec<u8>>> {
    let (first, last) = match medals {
        [medal] => (medal.achieved_at, medal.achieved_at),
        [first, .., last] => (first.achieved_at, last.achieved_at),
        [] => return Ok(None),
    };

    let mut surface =
        surfaces::raster_n32_premul((w as i32, h as i32)).wrap_err("Failed to create surface")?;

    {
        let mut root = SkiaBackend::new(surface.canvas(), w, h).into_drawing_area();

        let background = RGBColor(19, 43, 33);
        root.fill(&background)
            .wrap_err("Failed to fill background")?;

        let title_style = TextStyle::from(("sans-serif", 25_i32, FontStyle::Bold)).color(&WHITE);
        root = root
            .titled("Medal history", title_style)
            .wrap_err("Failed to draw title")?;

        let mut chart = ChartBuilder::on(&root)
            .margin(9)
            .x_label_area_size(20)
            .y_label_area_size(40)
            .build_cartesian_2d(Monthly(first..last), 0..medals.len())
            .wrap_err("Failed to build chart")?;

        // Mesh and labels
        chart
            .configure_mesh()
            .disable_mesh()
            .label_style(("sans-serif", 20, &WHITE))
            .axis_style(RGBColor(7, 18, 14))
            .axis_desc_style(("sans-serif", 20, FontStyle::Bold, &WHITE))
            .draw()
            .wrap_err("Failed to draw mesh and labels")?;

        // Draw area
        let area_style = RGBColor(2, 186, 213).mix(0.6).filled();
        let border_style = RGBColor(0, 208, 138).stroke_width(3);
        let counter = MedalCounter::new(medals);
        let series = AreaSeries::new(counter, 0, area_style).border_style(border_style);
        chart.draw_series(series).wrap_err("Failed to draw area")?;
    }

    let png_bytes = surface
        .image_snapshot()
        .encode(None, EncodedImageFormat::PNG, None)
        .wrap_err("Failed to encode image")?
        .to_vec();

    Ok(Some(png_bytes))
}

struct MedalCounter<'m> {
    count: usize,
    medals: &'m [MedalCompact],
}

impl<'m> MedalCounter<'m> {
    fn new(medals: &'m [MedalCompact]) -> Self {
        Self { count: 0, medals }
    }
}

impl Iterator for MedalCounter<'_> {
    type Item = (OffsetDateTime, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let date = self.medals.first()?.achieved_at;
        self.count += 1;
        self.medals = &self.medals[1..];

        Some((date, self.count))
    }
}
