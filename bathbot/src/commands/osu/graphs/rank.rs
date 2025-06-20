use std::iter;

use bathbot_macros::command;
use bathbot_model::command_fields::GameModeOption;
use bathbot_util::{constants::GENERAL_ISSUE, matcher, numbers::WithComma};
use eyre::{ContextCompat, Report, Result, WrapErr};
use plotters::{
    prelude::{ChartBuilder, Circle, IntoDrawingArea, SeriesLabelPosition},
    series::AreaSeries,
    style::{BLACK, Color, GREEN, RED, RGBColor, ShapeStyle, WHITE},
};
use plotters_backend::FontStyle;
use plotters_skia::SkiaBackend;
use rosu_v2::{prelude::OsuError, request::UserId};
use skia_safe::{EncodedImageFormat, surfaces};
use twilight_model::guild::Permissions;

use super::{Graph, GraphRank};
use crate::{
    commands::osu::{
        graphs::{GRAPH_RANK_DESC, H, W},
        user_not_found,
    },
    core::{
        Context,
        commands::{CommandOrigin, prefix::Args},
    },
    manager::redis::osu::{CachedUser, UserArgs, UserArgsError},
};

impl<'m> GraphRank<'m> {
    fn args(mode: Option<GameModeOption>, args: Args<'m>) -> Self {
        let mut name = None;
        let mut discord = None;

        for arg in args {
            if let Some(id) = matcher::get_mention_user(arg) {
                discord = Some(id);
            } else {
                name = Some(arg.into());
            }
        }

        Self {
            mode,
            name,
            discord,
            from: None,
            until: None,
        }
    }
}

#[command]
#[desc(GRAPH_RANK_DESC)]
#[usage("[username]")]
#[examples("peppy")]
#[group(Osu)]
async fn prefix_graphrank(msg: &Message, args: Args<'_>, perms: Option<Permissions>) -> Result<()> {
    let args = GraphRank::args(None, args);
    let orig = CommandOrigin::from_msg(msg, perms);

    super::graph(orig, Graph::Rank(args)).await
}

#[command]
#[desc(GRAPH_RANK_DESC)]
#[usage("[username]")]
#[examples("peppy")]
#[group(Taiko)]
async fn prefix_graphranktaiko(
    msg: &Message,
    args: Args<'_>,
    perms: Option<Permissions>,
) -> Result<()> {
    let args = GraphRank::args(Some(GameModeOption::Taiko), args);
    let orig = CommandOrigin::from_msg(msg, perms);

    super::graph(orig, Graph::Rank(args)).await
}

#[command]
#[desc(GRAPH_RANK_DESC)]
#[usage("[username]")]
#[examples("peppy")]
#[aliases("graphrankcatch")]
#[group(Catch)]
async fn prefix_graphrankctb(
    msg: &Message,
    args: Args<'_>,
    perms: Option<Permissions>,
) -> Result<()> {
    let args = GraphRank::args(Some(GameModeOption::Catch), args);
    let orig = CommandOrigin::from_msg(msg, perms);

    super::graph(orig, Graph::Rank(args)).await
}

#[command]
#[desc(GRAPH_RANK_DESC)]
#[usage("[username]")]
#[examples("peppy")]
#[group(Mania)]
async fn prefix_graphrankmania(
    msg: &Message,
    args: Args<'_>,
    perms: Option<Permissions>,
) -> Result<()> {
    let args = GraphRank::args(Some(GameModeOption::Mania), args);
    let orig = CommandOrigin::from_msg(msg, perms);

    super::graph(orig, Graph::Rank(args)).await
}

pub async fn rank_graph(
    orig: &CommandOrigin<'_>,
    user_id: UserId,
    user_args: UserArgs,
    from: Option<u8>,
    until: Option<u8>,
) -> Result<Option<(CachedUser, Vec<u8>)>> {
    fn draw_graph(user: &CachedUser, from: u8, until: u8) -> Result<Option<Vec<u8>>> {
        if user.rank_history.len() < 90 - from as usize {
            return Ok(None);
        }

        let history = &user.rank_history[90 - until as usize..90 - from as usize];

        let mut min = u32::MAX;
        let mut max = 0;

        let mut min_idx = 0;
        let mut max_idx = 0;

        for (&rank, i) in history.iter().zip(from as usize..) {
            let rank = rank.to_native();

            if rank == 0 {
                continue;
            }

            if rank < min {
                min = rank;
                min_idx = i;

                if rank > max {
                    max = rank;
                    max_idx = i;
                }
            } else if rank > max {
                max = rank;
                max_idx = i;
            }
        }

        let y_label_area_size = if max > 1_000_000 {
            85
        } else if max > 100_000 {
            80
        } else if max > 10_000 {
            75
        } else if max > 1000 {
            70
        } else if max > 100 {
            65
        } else if max > 10 {
            60
        } else {
            50
        };

        let (min, max) = (-(max as i32), -(min as i32));

        let mut surface = surfaces::raster_n32_premul((W as i32, H as i32))
            .wrap_err("Failed to create surface")?;

        {
            let root = SkiaBackend::new(surface.canvas(), W, H).into_drawing_area();

            let background = RGBColor(19, 43, 33);
            root.fill(&background)
                .wrap_err("Failed to fill background")?;

            let style: fn(RGBColor) -> ShapeStyle = |color| ShapeStyle {
                color: color.to_rgba(),
                filled: false,
                stroke_width: 1,
            };

            let mut chart = ChartBuilder::on(&root)
                .x_label_area_size(40)
                .y_label_area_size(y_label_area_size)
                .margin(10)
                .margin_left(6)
                .build_cartesian_2d(from as u32..(until as u32).saturating_sub(1), min..max)
                .wrap_err("Failed to build chart")?;

            chart
                .configure_mesh()
                .disable_y_mesh()
                .x_labels(20)
                .x_desc("Days ago")
                .x_label_formatter(&|x| format!("{}", (until + from) as u32 - *x))
                .y_label_formatter(&|y| format!("{}", -*y))
                .y_desc("Rank")
                .label_style(("sans-serif", 15, &WHITE))
                .bold_line_style(WHITE.mix(0.3))
                .axis_style(RGBColor(7, 18, 14))
                .axis_desc_style(("sans-serif", 16, FontStyle::Bold, &WHITE))
                .draw()
                .wrap_err("Failed to draw mesh")?;

            let data = (from as u32..)
                .zip(history.iter().map(|rank| -(rank.to_native() as i32)))
                .skip_while(|(_, rank)| *rank == 0)
                .take_while(|(_, rank)| *rank != 0);

            let area_style = RGBColor(2, 186, 213).mix(0.7).filled();
            let border_style = style(RGBColor(0, 208, 138)).stroke_width(3);
            let series = AreaSeries::new(data, min, area_style).border_style(border_style);
            chart.draw_series(series).wrap_err("Failed to draw area")?;

            let max_coords = (min_idx as u32, max);
            let circle = Circle::new(max_coords, 9_u32, style(GREEN).stroke_width(2));

            chart
                .draw_series(iter::once(circle))
                .wrap_err("Failed to draw max circle")?
                .label(format!("Peak: #{}", WithComma::new(-max)))
                .legend(|(x, y)| Circle::new((x, y), 5_u32, style(GREEN).stroke_width(2)));

            let min_coords = (max_idx as u32, min);
            let circle = Circle::new(min_coords, 9_u32, style(RED).stroke_width(2));

            chart
                .draw_series(iter::once(circle))
                .wrap_err("Failed to draw min circle")?
                .label(format!("Worst: #{}", WithComma::new(-min)))
                .legend(|(x, y)| Circle::new((x, y), 5_u32, style(RED).stroke_width(2)));

            let limit = (until - from) / 2 + from;

            let position = if min_idx >= limit as usize {
                SeriesLabelPosition::UpperLeft
            } else {
                SeriesLabelPosition::UpperRight
            };

            chart
                .configure_series_labels()
                .border_style(BLACK.stroke_width(2))
                .background_style(RGBColor(192, 192, 192))
                .position(position)
                .legend_area_size(13)
                .label_font(("sans-serif", 15, FontStyle::Bold))
                .draw()
                .wrap_err("Failed to draw legend")?;
        }

        let png_bytes = surface
            .image_snapshot()
            .encode(None, EncodedImageFormat::PNG, None)
            .wrap_err("Failed to encode image")?
            .to_vec();

        Ok(Some(png_bytes))
    }

    let user = match Context::redis().osu_user(user_args).await {
        Ok(user) => user,
        Err(UserArgsError::Osu(OsuError::NotFound)) => {
            let content = user_not_found(user_id).await;
            orig.error(content).await?;

            return Ok(None);
        }
        Err(err) => {
            let _ = orig.error(GENERAL_ISSUE).await;
            let err = Report::new(err).wrap_err("Failed to get user");

            return Err(err);
        }
    };

    let from_unwrapped = from.unwrap_or(0);
    let until_unwrapped = u8::max(until.unwrap_or(90), u8::min(from_unwrapped + 2, 90));

    let bytes = match draw_graph(&user, from_unwrapped, until_unwrapped) {
        Ok(Some(graph)) => graph,
        Ok(None) => {
            let mut content = format!(
                "`{name}` has no available rank data",
                name = user.username.as_str()
            );

            if from.is_some() || until.is_some() {
                content.push_str(" for this time range");
            }

            orig.error(content).await?;

            return Ok(None);
        }
        Err(err) => {
            let _ = orig.error(GENERAL_ISSUE).await;
            warn!(?err, "Failed to draw rank graph");

            return Ok(None);
        }
    };

    Ok(Some((user, bytes)))
}
