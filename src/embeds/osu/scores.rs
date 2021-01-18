use crate::{
    embeds::{osu, Author, EmbedData, Footer},
    pp::{Calculations, PPCalculator},
    unwind_error,
    util::{
        constants::{AVATAR_URL, MAP_THUMB_URL, OSU_BASE},
        datetime::how_long_ago,
        numbers::{with_comma, with_comma_u64},
        osu::grade_completion_mods,
        ScoreExt,
    },
};

use rosu::model::{Beatmap, GameMode, Score, User};
use std::fmt::Write;
use twilight_embed_builder::image_source::ImageSource;

pub struct ScoresEmbed {
    description: Option<&'static str>,
    fields: Vec<(String, String, bool)>,
    thumbnail: ImageSource,
    footer: Footer,
    author: Author,
    title: String,
    url: String,
}

impl ScoresEmbed {
    pub async fn new<'i, S>(user: &User, map: &Beatmap, scores: S, idx: usize) -> Self
    where
        S: Iterator<Item = &'i Score>,
    {
        let mut fields = Vec::with_capacity(4);
        for (i, score) in scores.enumerate() {
            let calculations = Calculations::all();
            let mut calculator = PPCalculator::new().score(score).map(map);
            if let Err(why) = calculator.calculate(calculations).await {
                unwind_error!(warn, why, "Error while calculating pp for scores: {}");
            }
            let stars = osu::get_stars(calculator.stars().unwrap_or(0.0));
            let pp = osu::get_pp(calculator.pp(), calculator.max_pp());
            let mut name = format!(
                "**{idx}.** {grade}\t[{stars}]\t{score}\t({acc})",
                idx = idx + i + 1,
                grade = grade_completion_mods(&score, map),
                stars = stars,
                score = with_comma_u64(score.score as u64),
                acc = score.acc_string(map.mode),
            );
            if map.mode == GameMode::MNA {
                let _ = write!(name, "\t{}", osu::get_keys(score.enabled_mods, map));
            }
            let value = format!(
                "{pp}\t[ {combo} ]\t {hits}\t{ago}",
                pp = pp,
                combo = osu::get_combo(score, map),
                hits = score.hits_string(map.mode),
                ago = how_long_ago(&score.date)
            );
            fields.push((name, value, false));
        }
        let footer = Footer::new(format!("{:?} map by {}", map.approval_status, map.creator))
            .icon_url(format!("{}{}", AVATAR_URL, map.creator_id));
        let author_text = format!(
            "{name}: {pp}pp (#{global} {country}{national})",
            name = user.username,
            pp = with_comma(user.pp_raw),
            global = with_comma_u64(user.pp_rank as u64),
            country = user.country,
            national = user.pp_country_rank
        );
        let author = Author::new(author_text)
            .url(format!("{}u/{}", OSU_BASE, user.user_id))
            .icon_url(format!("{}{}", AVATAR_URL, user.user_id));
        let description = match fields.is_empty() {
            true => Some("No scores found"),
            false => None,
        };
        Self {
            description,
            footer,
            thumbnail: ImageSource::url(format!("{}{}l.jpg", MAP_THUMB_URL, map.beatmapset_id))
                .unwrap(),
            title: map.to_string(),
            url: format!("{}b/{}", OSU_BASE, map.beatmap_id),
            fields,
            author,
        }
    }
}

impl EmbedData for ScoresEmbed {
    fn description(&self) -> Option<&str> {
        self.description
    }
    fn fields(&self) -> Option<Vec<(String, String, bool)>> {
        Some(self.fields.clone())
    }
    fn url(&self) -> Option<&str> {
        Some(&self.url)
    }
    fn title(&self) -> Option<&str> {
        Some(&self.title)
    }
    fn footer(&self) -> Option<&Footer> {
        Some(&self.footer)
    }
    fn author(&self) -> Option<&Author> {
        Some(&self.author)
    }
    fn thumbnail(&self) -> Option<&ImageSource> {
        Some(&self.thumbnail)
    }
}
