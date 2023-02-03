use std::iter;

use bathbot_util::{
    constants::{GENERAL_ISSUE, OSU_API_ISSUE},
    numbers::WithComma,
};
use eyre::{Report, Result, WrapErr};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use plotters::{
    prelude::{BitMapBackend, ChartBuilder, Circle, IntoDrawingArea, SeriesLabelPosition},
    series::AreaSeries,
    style::{Color, RGBColor, ShapeStyle, BLACK, GREEN, RED, WHITE},
};
use plotters_backend::FontStyle;
use rosu_v2::{prelude::OsuError, request::UserId};

use crate::{
    commands::osu::{
        graphs::{H, LEN, W},
        user_not_found,
    },
    core::{commands::CommandOrigin, Context},
    manager::redis::{
        osu::{User, UserArgs},
        RedisData,
    },
};

pub async fn rank_graph(
    ctx: &Context,
    orig: &CommandOrigin<'_>,
    user_id: UserId,
    user_args: UserArgs,
) -> Result<Option<(RedisData<User>, Vec<u8>)>> {
    let user = match ctx.redis().osu_user(user_args).await {
        Ok(user) => user,
        Err(OsuError::NotFound) => {
            let content = user_not_found(ctx, user_id).await;
            orig.error(ctx, content).await?;

            return Ok(None);
        }
        Err(err) => {
            let _ = orig.error(ctx, OSU_API_ISSUE).await;
            let err = Report::new(err).wrap_err("failed to get user");

            return Err(err);
        }
    };

    fn draw_graph(user: &RedisData<User>) -> Result<Option<Vec<u8>>> {
        let mut buf = vec![0; LEN * 3];

        let history = match user {
            RedisData::Original(user) if user.rank_history.is_empty() => return Ok(None),
            RedisData::Original(user) => user.rank_history.as_slice(),
            RedisData::Archived(user) if user.rank_history.is_empty() => return Ok(None),
            RedisData::Archived(user) => user.rank_history.as_slice(),
        };

        let history_len = history.len();

        let mut min = u32::MAX;
        let mut max = 0;

        let mut min_idx = 0;
        let mut max_idx = 0;

        for (i, &rank) in history.iter().enumerate() {
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
            75
        } else if max > 10_000 {
            65
        } else if max > 100 {
            50
        } else {
            40
        };

        let (min, max) = (-(max as i32), -(min as i32));

        {
            let root = BitMapBackend::with_buffer(&mut buf, (W, H)).into_drawing_area();
            let background = RGBColor(19, 43, 33);
            root.fill(&background)
                .wrap_err("failed to fill background")?;

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
                .build_cartesian_2d(0_u32..history_len.saturating_sub(1) as u32, min..max)
                .wrap_err("failed to build chart")?;

            chart
                .configure_mesh()
                .disable_y_mesh()
                .x_labels(20)
                .x_desc("Days ago")
                .x_label_formatter(&|x| format!("{}", 90 - *x))
                .y_label_formatter(&|y| format!("{}", -*y))
                .y_desc("Rank")
                .label_style(("sans-serif", 15, &WHITE))
                .bold_line_style(WHITE.mix(0.3))
                .axis_style(RGBColor(7, 18, 14))
                .axis_desc_style(("sans-serif", 16, FontStyle::Bold, &WHITE))
                .draw()
                .wrap_err("failed to draw mesh")?;

            let data = (0..).zip(history.iter().map(|rank| -(*rank as i32)));

            let area_style = RGBColor(2, 186, 213).mix(0.7).filled();
            let border_style = style(RGBColor(0, 208, 138)).stroke_width(3);
            let series = AreaSeries::new(data, min, area_style).border_style(border_style);
            chart.draw_series(series).wrap_err("failed to draw area")?;

            let max_coords = (min_idx as u32, max);
            let circle = Circle::new(max_coords, 9_u32, style(GREEN));

            chart
                .draw_series(iter::once(circle))
                .wrap_err("failed to draw max circle")?
                .label(format!("Peak: #{}", WithComma::new(-max)))
                .legend(|(x, y)| Circle::new((x, y), 5_u32, style(GREEN)));

            let min_coords = (max_idx as u32, min);
            let circle = Circle::new(min_coords, 9_u32, style(RED));

            chart
                .draw_series(iter::once(circle))
                .wrap_err("failed to draw min circle")?
                .label(format!("Worst: #{}", WithComma::new(-min)))
                .legend(|(x, y)| Circle::new((x, y), 5_u32, style(RED)));

            let position = if min_idx <= 70 {
                SeriesLabelPosition::UpperRight
            } else if max_idx > 70 {
                SeriesLabelPosition::UpperLeft
            } else {
                SeriesLabelPosition::LowerRight
            };

            chart
                .configure_series_labels()
                .border_style(BLACK.stroke_width(2))
                .background_style(RGBColor(192, 192, 192))
                .position(position)
                .legend_area_size(13)
                .label_font(("sans-serif", 15, FontStyle::Bold))
                .draw()
                .wrap_err("failed to draw legend")?;
        }

        // Encode buf to png
        let mut png_bytes: Vec<u8> = Vec::with_capacity(LEN);
        let png_encoder = PngEncoder::new(&mut png_bytes);

        png_encoder
            .write_image(&buf, W, H, ColorType::Rgb8)
            .wrap_err("failed to encode image")?;

        Ok(Some(png_bytes))
    }

    let bytes = match draw_graph(&user) {
        Ok(Some(graph)) => graph,
        Ok(None) => {
            let content = format!(
                "`{name}` has no available rank data :(",
                name = user.username()
            );

            orig.error(ctx, content).await?;

            return Ok(None);
        }
        Err(err) => {
            let _ = orig.error(ctx, GENERAL_ISSUE).await;
            warn!("{:?}", err.wrap_err("failed to draw rank graph"));

            return Ok(None);
        }
    };

    Ok(Some((user, bytes)))
}
